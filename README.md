# PurpleOps — console purple team mobile (Pixel 7a / chroot Kali)

Application Android qui pilote le **2e chroot Kali** (`/data/local/kali`) du terminal
mobile SOC. Objectif : lancer des outils offensifs depuis l'écran du téléphone, sans
clavier ni ligne de commande, **strictement contre tes propres machines** du homelab
(toutes sous agent Wazuh) pour mesurer la couverture de détection (MITRE T1110, etc.).

> Périmètre d'usage : **tes machines uniquement**. Cet outil exécute des commandes root
> dans Kali ; ne l'utilise que sur des cibles qui t'appartiennent.

## Ce que fait la V1

- Champ **cible** (IP / host / CIDR), mémorisé entre les lancements.
- **Boutons presets** (nmap -sV, -sC, ping sweep, quick, full, vuln, whoami…) — `{T}`
  est remplacé par la cible.
- **Commande libre** : tape n'importe quelle commande Kali (pipes inclus) ; `{T}` y est
  aussi substitué si présent.
- **Console** de sortie en direct (streaming), boutons STOP / CLEAR.

### Fonctionnement technique

- Exécution via `su` (KernelSU) → `chroot /data/local/kali` en environnement vierge,
  exactement comme le raccourci `kali-shell` de Termux.
- Le chroot est monté (idempotent) à chaque commande via `/data/local/kali-start.sh`.
- Aucune dépendance externe (ProcessBuilder + flux ligne à ligne).
- Presets éditables **sans recompiler** : dépose un `/data/local/soc/presets.json`
  (lu en priorité), sinon le JSON embarqué (`app/src/main/assets/presets.json`) sert de défaut.

Format d'un preset :
```json
{ "label": "nmap -sV", "cmd": "nmap -sV {T}", "needsTarget": true }
```

## Build

### Option A — GitHub Actions (recommandé, zéro install)
1. Pousse ce dossier dans un dépôt GitHub (`git init`, commit, push sur `main`).
2. L'onglet **Actions** lance `build-apk` → télécharge l'artefact **PurpleOps-debug-apk**.
3. L'APK est signé avec la clé *debug* → installable directement (sources inconnues).

### Option B — Android Studio (local)
Ouvre le dossier `PurpleOps/` dans Android Studio (il génère le wrapper), puis Run/Build.

### Option C — Gradle en ligne de commande
```
gradle wrapper --gradle-version 8.10.2
./gradlew assembleDebug
# APK : app/build/outputs/apk/debug/app-debug.apk
```

## Installation sur le téléphone
- Transfère `app-debug.apk` (ADB `adb install app-debug.apk`, ou copie + explorateur).
- Autorise l'installation de sources inconnues pour l'installateur.
- Au 1er lancement d'une commande, **KernelSU** demande d'accorder `su` à PurpleOps → Accorder.

## Traçabilité SOC (important pour la soutenance)
- L'installation de l'APK = fichier dans `/data/app` → captée par le **FIM règle 554**
  de l'agent 027 (groupe `Android`).
- Les outils lancés restent des `exec` dans le chroot (même noyau) → la traçabilité dpkg
  (2902/2904) et le FIM historique de l'étape 3 s'appliquent.
- Angle mort connu : l'exécution de commande en temps réel (quel binaire, quels arguments)
  n'est pas tracée tant qu'on n'a pas d'auditd execve dans le chroot (voir roadmap V2).

## Roadmap (on commence petit, on consolide)

- **V1 (actuelle)** : presets + commande libre + console. Interaction manuelle avec Kali.
- **V2** : catalogue d'outils par catégories (recon / brute / exploit), **journal des runs**
  écrit dans un fichier lu par Wazuh (trace native des commandes lancées = comble l'angle
  mort ci-dessus, sans auditd), sauvegarde / partage de la sortie, favoris.
- **V3** : **API locale** embarquée dans l'APK (petit serveur HTTP) — `POST /run {cmd}`,
  bind sur l'IP **Tailscale uniquement**, **token d'auth** obligatoire, chaque appel **loggé**
  pour le SOC. Permet un pilotage programmatique.
- **V4** : **serveur MCP** (Python, dans le chroot Debian ou sur le manager) qui enveloppe
  l'API V3 → **Claude Desktop pilote Kali** (rejoint la Phase 4 « Triage IA + MCP » du PFE).

### Note de sécurité pour V3/V4
Une API qui exécute des commandes = RCE par conception. Règles non négociables prévues :
écoute sur l'IP Tailscale (jamais `0.0.0.0`), token/secret, allowlist de commandes ou de
cibles, endpoint **désactivé par défaut** (activé seulement pendant un exercice), et
journalisation systématique vers Wazuh. À relire comme un changement touchant l'exposition
réseau avant toute mise en service.
