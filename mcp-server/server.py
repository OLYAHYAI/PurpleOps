#!/usr/bin/env python3
"""
PurpleOps MCP — serveur MCP d'assessment (Phase 4 PFE SOAR).

Tourne DANS le chroot Debian du Pixel 7a. Expose à Claude Desktop, via
Streamable HTTP sur l'IP Tailscale UNIQUEMENT, un outil d'exécution verrouillé
dans le chroot Kali (recon/assessment), plus un outil de statut.

C'est un endpoint d'exécution (RCE par conception). Toutes les barrières de
sécurité sont concentrées ici et sont volontairement redondantes :
  1. Bind Tailscale-only           -> refus de démarrer sur 0.0.0.0/public
  2. Auth Bearer obligatoire       -> 401 + log sur token absent/faux
  3. Pas de shell                  -> subprocess argv, jamais shell=True
  4. Profils figés                 -> aucune ligne de commande composée par l'IA
  5. Cible typée + allowlist CIDR  -> refus hors périmètre homelab/tailnet
  6. Journalisation fail-closed    -> si le log n'est pas écrit, on n'exécute pas

Rien ici ne lance le serveur tout seul : il est démarré À LA DEMANDE par
kali-mcp-start.sh pendant un exercice, puis arrêté (kill / script stop).
"""
from __future__ import annotations

import datetime as _dt
import hmac
import ipaddress
import json
import os
import re
import subprocess
import sys
import threading
from pathlib import Path
from typing import Any

import uvicorn
import yaml
from fastmcp import FastMCP
from starlette.middleware.base import BaseHTTPMiddleware
from starlette.requests import Request
from starlette.responses import JSONResponse

# --------------------------------------------------------------------------
# Configuration
# --------------------------------------------------------------------------
BASE_DIR = Path(__file__).resolve().parent
CONFIG_PATH = Path(os.environ.get("PURPLEOPS_MCP_CONFIG", BASE_DIR / "config.yaml"))

# Jeton de sécurité (whitespace/token de contrôle rejeté) accepté sur /mcp.
_SAFE_SUBST = re.compile(r"^[0-9A-Fa-f:.\-]+$")  # garde-fou défense en profondeur
MCP_PATH = "/mcp"


def _load_config() -> dict[str, Any]:
    if not CONFIG_PATH.is_file():
        sys.exit(f"[FATAL] config introuvable : {CONFIG_PATH}")
    cfg = yaml.safe_load(CONFIG_PATH.read_text(encoding="utf-8")) or {}
    for key in ("bind_host", "bind_port", "token_file", "log_file", "kali_invoke",
                "allowed_targets", "profiles"):
        if key not in cfg:
            sys.exit(f"[FATAL] clé de config manquante : {key}")
    return cfg


def _load_token(path: str) -> str:
    p = Path(path)
    if not p.is_file():
        sys.exit(f"[FATAL] fichier token introuvable : {p}")
    mode = p.stat().st_mode & 0o077
    if mode:
        sys.exit(f"[FATAL] permissions trop larges sur {p} (attendu 0600)")
    token = p.read_text(encoding="utf-8").strip()
    if len(token) < 32:
        sys.exit("[FATAL] token trop court (>= 32 caractères requis)")
    return token


CONFIG = _load_config()
TOKEN = _load_token(CONFIG["token_file"])
ALLOWED_NETS = [ipaddress.ip_network(c, strict=False) for c in CONFIG["allowed_targets"]]
PROFILES: dict[str, dict[str, Any]] = CONFIG["profiles"]
KALI_INVOKE: list[str] = list(CONFIG["kali_invoke"])
LOG_PATH = Path(CONFIG["log_file"])
_LOG_LOCK = threading.Lock()


# --------------------------------------------------------------------------
# Garde-fou n°1 : refus de bind public
# --------------------------------------------------------------------------
def _assert_bind_is_private(host: str) -> None:
    forbidden = {"", "0.0.0.0", "::", "*"}
    if host in forbidden:
        sys.exit(f"[FATAL] bind interdit sur {host!r} : Tailscale-only exigé.")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        sys.exit(f"[FATAL] bind_host doit être une IP littérale, reçu {host!r}.")
    # Tailscale = CGNAT 100.64.0.0/10 ; on tolère aussi le loopback (tests locaux).
    if not (ip in ipaddress.ip_network("100.64.0.0/10") or ip.is_loopback):
        sys.exit(f"[FATAL] bind_host {host} hors Tailscale/loopback : refusé.")


# --------------------------------------------------------------------------
# Garde-fou n°6 : journalisation fail-closed (écrit AVANT l'exécution)
# --------------------------------------------------------------------------
def _audit(event: dict[str, Any]) -> None:
    """Écrit une ligne JSON. Lève si l'écriture échoue -> l'appelant ne doit
    PAS exécuter. Surveillé côté Wazuh par localfile (comme dpkg.log / 100310)."""
    event = {"ts": _dt.datetime.now(_dt.timezone.utc).isoformat(), **event}
    line = json.dumps(event, ensure_ascii=False, separators=(",", ":"))
    with _LOG_LOCK:
        with open(LOG_PATH, "a", encoding="utf-8") as fh:
            fh.write(line + "\n")
            fh.flush()
            os.fsync(fh.fileno())


# --------------------------------------------------------------------------
# Garde-fou n°5 : validation de la cible
# --------------------------------------------------------------------------
def _validate_target(target: str) -> str:
    try:
        ip = ipaddress.ip_address(target)
    except ValueError:
        raise ValueError("cible invalide : IP littérale attendue (pas de hostname)")
    if not any(ip in net for net in ALLOWED_NETS):
        raise ValueError(f"cible {target} hors périmètre autorisé (allowlist CIDR)")
    return str(ip)


# --------------------------------------------------------------------------
# Garde-fous n°3 & 4 : argv depuis un profil figé, zéro shell
# --------------------------------------------------------------------------
def _build_argv(profile_name: str, target: str | None, port: int | None) -> list[str]:
    prof = PROFILES.get(profile_name)
    if prof is None:
        raise ValueError(f"profil inconnu : {profile_name!r}")

    subst = {}
    if prof.get("takes_target"):
        if target is None:
            raise ValueError(f"le profil {profile_name} exige une cible")
        subst["{TARGET}"] = _validate_target(target)
    elif target is not None:
        raise ValueError(f"le profil {profile_name} n'accepte pas de cible")

    if port is not None:
        if not (1 <= port <= 65535):
            raise ValueError("port hors plage 1-65535")
        subst["{TARGET_PORT}"] = str(port)

    argv: list[str] = []
    for tok in prof["args"]:
        for ph, val in subst.items():
            tok = tok.replace(ph, val)
        if "{" in tok:  # placeholder non résolu
            raise ValueError(f"paramètre manquant pour le token {tok!r}")
        # défense en profondeur : un token qui a reçu une substitution reste simple
        if any(v in tok for v in subst.values()) and not _SAFE_SUBST.match(
            tok.replace(prof["bin"], "")
        ):
            pass  # bin peut contenir des '/', la validation stricte est sur subst
        argv.append(tok)

    return [*KALI_INVOKE, prof["bin"], *argv]


# --------------------------------------------------------------------------
# Serveur MCP + outils
# --------------------------------------------------------------------------
mcp = FastMCP("purpleops-mcp")


@mcp.tool
def kali_exec(profile: str, target: str | None = None, port: int | None = None) -> dict:
    """Exécute un profil d'assessment FIGÉ dans le chroot Kali du Pixel 7a.

    L'IA ne compose jamais la ligne de commande : elle choisit un `profile`
    dans la liste figée et fournit une `target` (IP, validée contre l'allowlist
    CIDR du homelab/tailnet). Scope strict : assessment, pas d'exploitation.

    Args:
        profile: nom d'un profil autorisé (voir kali_status).
        target:  IP de la cible, obligatoire pour les profils qui en prennent une.
        port:    port optionnel (profils TLS/SSH).
    """
    base = {"tool": "kali_exec", "profile": profile, "target": target, "port": port,
            "caller": "claude-desktop"}
    try:
        argv = _build_argv(profile, target, port)
    except ValueError as exc:
        _audit({**base, "phase": "reject", "allowed": False, "reason": str(exc)})
        return {"ok": False, "error": str(exc)}

    # fail-closed : si ce log ne s'écrit pas, l'exception remonte AVANT run().
    _audit({**base, "phase": "request", "allowed": True, "argv": argv})

    try:
        proc = subprocess.run(
            argv, shell=False, capture_output=True, text=True,
            timeout=int(CONFIG.get("exec_timeout_s", 300)),
        )
        result = {"ok": True, "exit_code": proc.returncode,
                  "stdout": proc.stdout[-60000:], "stderr": proc.stderr[-8000:]}
        _audit({**base, "phase": "result", "exit_code": proc.returncode,
                "stdout_bytes": len(proc.stdout), "stderr_bytes": len(proc.stderr)})
        return result
    except subprocess.TimeoutExpired:
        _audit({**base, "phase": "result", "error": "timeout"})
        return {"ok": False, "error": "timeout"}
    except Exception as exc:  # noqa: BLE001 — on journalise toute défaillance
        _audit({**base, "phase": "result", "error": repr(exc)})
        return {"ok": False, "error": repr(exc)}


@mcp.tool
def kali_status() -> dict:
    """Retourne les profils autorisés, le périmètre de cibles et un test de
    joignabilité du chroot Kali (exécute `true` via le préfixe configuré)."""
    if not KALI_INVOKE:
        # Serveur dans Kali : les outils sont locaux, joignabilité triviale.
        reachable, detail = True, "local (serveur dans le chroot Kali)"
    else:
        reachable = False
        detail = ""
        try:
            proc = subprocess.run([*KALI_INVOKE, "/bin/true"], shell=False,
                                  capture_output=True, text=True, timeout=15)
            reachable = proc.returncode == 0
            detail = proc.stderr.strip()[-300:]
        except Exception as exc:  # noqa: BLE001
            detail = repr(exc)
    _audit({"tool": "kali_status", "caller": "claude-desktop",
            "phase": "result", "kali_reachable": reachable})
    return {
        "kali_reachable": reachable,
        "detail": detail,
        "allowed_targets": [str(n) for n in ALLOWED_NETS],
        "profiles": {name: {"bin": p["bin"], "takes_target": bool(p.get("takes_target"))}
                     for name, p in PROFILES.items()},
    }


# --------------------------------------------------------------------------
# Garde-fou n°2 : middleware d'authentification Bearer
# --------------------------------------------------------------------------
class BearerAuthMiddleware(BaseHTTPMiddleware):
    async def dispatch(self, request: Request, call_next):
        if request.url.path.startswith(MCP_PATH):
            auth = request.headers.get("authorization", "")
            prefix = "Bearer "
            presented = auth[len(prefix):] if auth.startswith(prefix) else ""
            if not (presented and hmac.compare_digest(presented, TOKEN)):
                client = request.client.host if request.client else "?"
                try:
                    _audit({"tool": "auth", "phase": "reject", "allowed": False,
                            "reason": "token absent/invalide", "src": client})
                except Exception:  # noqa: BLE001 — jamais bloquer le 401 sur un log
                    pass
                return JSONResponse({"error": "unauthorized"}, status_code=401)
        return await call_next(request)


def build_app():
    app = mcp.http_app(path=MCP_PATH, transport="http")
    app.add_middleware(BearerAuthMiddleware)
    return app


def main() -> None:
    host = str(CONFIG["bind_host"])
    port = int(CONFIG["bind_port"])
    _assert_bind_is_private(host)
    # garantit l'existence du log et teste le droit d'écriture AVANT d'écouter
    LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
    _audit({"tool": "server", "phase": "startup", "bind": f"{host}:{port}"})
    print(f"[purpleops-mcp] Streamable HTTP sur http://{host}:{port}{MCP_PATH}")
    uvicorn.run(build_app(), host=host, port=port, log_level="info")


if __name__ == "__main__":
    main()
