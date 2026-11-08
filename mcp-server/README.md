# PurpleOps MCP — serveur d'assessment (Phase 4 PFE SOAR)

Serveur MCP **Streamable HTTP** tournant dans le chroot **Kali** du Pixel 7a
(les outils d'assessment y sont locaux). Expose à Claude Desktop, sur l'**IP
Tailscale uniquement**, un outil d'exécution verrouillé + un outil de statut.
Son journal (`/var/log/purpleops-mcp.log` dans Kali) est lu par l'agent Wazuh
côté Debian via le bind ro `/android/kali` déjà en place.

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

## Pilotage de l'écran du téléphone (outils `phone_*`)
Pour que Claude Desktop puisse manipuler le téléphone, à la manière de l'extension Chrome :

| Outil | Effet |
| --- | --- |
| `phone_read_screen` | éléments affichés (texte, bouton, champ) avec leur **centre en pixels écran**, via uiautomator |
| `phone_screenshot` | capture d'écran (réduite en JPEG si Pillow est installé, sinon PNG) + facteur de conversion des coordonnées |
| `phone_tap` / `phone_swipe` | toucher / glisser (appui long : même point, ~800 ms) |
| `phone_key` | back, home, recents, enter, delete, flèches… (liste fermée) |
| `phone_type` | texte dans le champ actif, accents et emoji compris (aide Java de l'APK) |

**Garde-fous**
- **Décision sur le téléphone** : onglet Écran → « Autoriser le pilotage par l'IA » (décoché
  par défaut). Coché, `screenctl --ctl` ouvre une prise Unix root 0600 dans
  `/data/local/kali/opt/purpleops-mcp/screenctl.sock` (vue `/opt/purpleops-mcp/screenctl.sock`
  depuis le chroot). Décoché : pas de prise, les outils refusent avec un message clair.
- Pas de port réseau supplémentaire : la prise est locale ; l'accès distant reste celui du
  MCP (Tailscale + Bearer).
- Coordonnées bornées à l'écran, touches en liste fermée, texte ≤ 4096 octets.
- **Double journal** : audit JSONL fail-closed du MCP (comme `kali_exec`) + journal SOC
  `/data/local/soc/screenctl.log` (« ia tap 540 1200 »…). Le texte tapé n'est journalisé
  que par sa longueur ; les captures ne sont pas conservées.

## Démarrage depuis l'appli (sans commande)
PurpleOps → **Réglages → Serveur MCP PurpleOps** :
- l'appli cherche `opt/purpleops-mcp/server.py` dans `/data/local/kali` puis
  `/data/local/debian` et **affiche le chroot où il est installé** ;
- DÉMARRER / ARRÊTER, et **démarrage automatique à l'ouverture de l'appli**
  (case cochée par défaut, décochable) ;
- **Mettre à jour le serveur MCP** : récupère le dernier `server.py` depuis GitHub
  (`purpleops-mcp-update.sh` : config et jeton conservés, syntaxe vérifiée), puis
  redémarre. Le code du MCP vit dans le chroot et ne suit pas les mises à jour de l'APK ;
- lancement : script de montage du chroot (`kali-start.sh` / `debian-start.sh`),
  puis `kali-mcp-start.sh` dans le chroot, détaché (`setsid`) et sorti du cgroup
  de l'appli (il survit à sa fermeture). Sortie : `<chroot>/var/log/purpleops-mcp.out`.

Les barrières restent celles du serveur (bind Tailscale, Bearer, journal fail-closed) :
si Tailscale est coupé, `kali-mcp-start.sh` refuse de démarrer et la raison s'affiche.

## Déploiement (dans le chroot Kali)
- Code dans `/opt/purpleops-mcp` (= `/data/local/kali/opt/...` côté Android).
- `kali_invoke: []` (outils locaux, aucun saut de chroot).
- Token `/etc/purpleops-mcp/token` (0600), jamais commité.
- Connectivité Claude Desktop → `100.126.112.23:8765` via le tailnet.

## Tests locaux (déjà passés en conteneur)
Refus hors allowlist, refus injection cible, refus profil inconnu, fail-closed,
refus bind public, `401` sans/mauvais token, `200` avec bon token.
