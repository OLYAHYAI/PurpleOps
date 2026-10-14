# PurpleOps MCP — serveur d'assessment (Phase 4 PFE SOAR)

Serveur MCP **Streamable HTTP** tournant dans le chroot Debian du Pixel 7a.
Expose à Claude Desktop, sur l'**IP Tailscale uniquement**, un outil d'exécution
verrouillé dans le chroot Kali (recon/assessment) + un outil de statut.

> Endpoint d'exécution = RCE par conception. Les barrières sont concentrées dans
> `server.py` et volontairement redondantes.

## Barrières de sécurité
1. **Bind Tailscale-only** — refus de démarrer sur `0.0.0.0`/`::`/IP publique.
2. **Auth Bearer** obligatoire — `401` + log sur token absent/faux (compare constante).
3. **Zéro shell** — `subprocess` argv, jamais `shell=True`.
4. **Profils figés** — l'IA choisit un profil, ne compose aucune commande.
5. **Cible typée + allowlist CIDR** — IP littérale validée contre le périmètre homelab.
6. **Journal fail-closed** — si l'audit JSONL ne s'écrit pas (`fsync`), on n'exécute pas.

Transport : HTTP+SSE (l'ancien transport MCP à deux endpoints) est **déprécié** depuis
la révision 2025-03-26 de la spec ; on utilise donc Streamable HTTP. Tailscale
(WireGuard) chiffre déjà le flux → HTTP simple sur le tailnet, pas de TLS self-signed.

## Fichiers
- `server.py` — le serveur et les deux outils (`kali_exec`, `kali_status`).
- `config.yaml` — bind, token, log, préfixe d'entrée Kali, allowlist, profils.
- `kali-mcp-start.sh` — lancement à la demande (pas de systemd dans le chroot).
- `requirements.txt` — `fastmcp`, `pyyaml`, `uvicorn`.

## Journalisation → Wazuh
Chaque appel écrit une ligne JSON dans `log_file` (phases `request`/`result`/`reject`).
À surveiller par l'agent 027 via `localfile` (même modèle que `dpkg.log` / règle 100310).
Le décodeur + les règles custom côté manager suivent la **procédure de changement**
(branche Git, backup, `wazuh-analysisd -t`, restart, non-régression) — étape ultérieure.

## À confirmer sur le téléphone (point de contrôle)
- `kali_invoke` : mécanisme réel d'entrée dans le chroot Kali depuis le chroot Debian
  (mounts, droits root). Valeur par défaut `["chroot","/data/local/kali"]` à valider.
- Chemins `token_file` / `log_file` et emplacement du code (`/opt/purpleops-mcp`).
- Connectivité Claude Desktop → `100.126.112.23:8765` via le tailnet.

## Tests locaux (déjà passés en conteneur)
Refus hors allowlist, refus injection cible, refus profil inconnu, fail-closed,
refus bind public, `401` sans/mauvais token, `200` avec bon token.
