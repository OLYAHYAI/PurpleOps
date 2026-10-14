#!/bin/sh
# Lancement À LA DEMANDE du serveur PurpleOps MCP, DANS le chroot Kali.
# Le chroot n'a pas de systemd : lancement par script (comme kali-start.sh).
# À coller/exécuter TOI-MÊME dans le chroot Kali. Ne jamais mettre `exit` à la suite.

set -eu

TAILSCALE_IP="100.126.112.23"
APP_DIR="/opt/purpleops-mcp"          # emplacement du code dans le chroot Debian
TOKEN_FILE="/etc/purpleops-mcp/token"

# 1) Interface Tailscale up ? sinon on NE démarre PAS (pas de fallback public).
if ! ip -4 addr show 2>/dev/null | grep -q "$TAILSCALE_IP"; then
  echo "[STOP] IP Tailscale $TAILSCALE_IP absente — Tailscale down. Abandon." >&2
  return 1 2>/dev/null || exit 1
fi

# 2) Token présent et en 0600 ?
if [ ! -f "$TOKEN_FILE" ]; then
  echo "[STOP] token absent : $TOKEN_FILE" >&2
  return 1 2>/dev/null || exit 1
fi

# 3) Lancement (premier plan ; Ctrl-C pour arrêter en fin d'exercice).
cd "$APP_DIR"
# venv si présent (recommandé), sinon python3 système.
PY="$APP_DIR/venv/bin/python"
[ -x "$PY" ] || PY=python3
echo "[purpleops-mcp] démarrage sur $TAILSCALE_IP:8765 — Ctrl-C pour arrêter"
exec "$PY" "$APP_DIR/server.py"
