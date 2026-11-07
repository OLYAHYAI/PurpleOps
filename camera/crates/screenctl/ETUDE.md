# screenctl — étude courte (scrcpy maison, lien USB)

But : voir et piloter l'écran du Pixel 7a depuis le PC Windows, **par câble USB-C
uniquement**, sans `adb` et sans Wi-Fi. Service côté téléphone intégré à PurpleOps
(Rust, comme `p900ctl`), client côté PC en Rust. Désactivé par défaut.

## 1. Transport : partage de connexion USB

Android expose, quand on active « Partage de connexion par USB », une interface
réseau point à point (RNDIS → `rndis0`, ou NCM → `ncm0` sur Android récent). Le
téléphone y prend l'adresse de passerelle **192.168.42.129** et le PC reçoit une
adresse par DHCP (192.168.42.x). C'est un lien direct câble : rien ne transite par
le Wi-Fi ni par Tailscale.

Le service **lie sa socket à cette seule interface** (SO_BINDTODEVICE, root — même
mécanisme que `--iface wlan0` de `p900ctl`) et à l'IP tether. Jamais `0.0.0.0`,
jamais `wlan0`. Débit USB 2.0 ≈ 480 Mb/s, largement au-dessus d'un flux H.264
d'écran : le câble n'est pas le facteur limitant.

Côté PC, le client se connecte à `192.168.42.129:<port>`.

## 2. Capture : root plutôt que MediaProjection

| Critère | **Root (screencap/screenrecord)** choisi | MediaProjection |
| --- | --- | --- |
| Glue Java/Surface/MediaCodec | aucune — un seul binaire Rust | nécessaire (service 1er plan + Surface) |
| Consentement | aucun (on est déjà root) | popup système à chaque session |
| H.264 matériel | `screenrecord --output-format=h264 --output=-` le donne gratuitement | via MediaCodec, à câbler soi-même |
| Cohérence projet | identique à `p900ctl` (root KernelSU, std pur) | rompt le modèle |

**Décision : voie root.** Le projet est déjà entièrement root (KernelSU), la
philosophie est « un service Rust autonome ». `screencap` fournit des trames brutes
pour l'incrément 1 ; `screenrecord` fournit du H.264 encodé matériellement pour
l'incrément vidéo, sans écrire une ligne de Java.

Limites connues de `screenrecord` (à gérer) :
- coupure à ~3 min par appel → on relancera le process de façon transparente
  (petit trou d'image à la jointure) ;
- surfaces sécurisées (DRM, écran de verrouillage) → noir, acceptable ici ;
- rotation figée au démarrage de la capture.

Montée en gamme « do it right » (plus tard, si les limites gênent) : un programme
lancé par `app_process` appelant les API cachées `SurfaceControl` + `MediaCodec`
— c'est ce que fait scrcpy — pour une durée illimitée et zéro trou. MediaProjection
reste le plan B si un jour on veut un mode **sans** root.

## 3. Codec : brut → MJPEG → H.264 (incréments mesurables)

1. **Brut RGBA** (incrément 1) : `screencap` renvoie l'image telle quelle. Aucun
   décodage ni des deux côtés — on valide capture + affichage + toucher d'abord.
2. **MJPEG** (incrément 2a) : trames JPEG, diagnostic simple, mesure du gain de
   bande passante vs brut.
3. **H.264** (incrément 2b) : `screenrecord` en flux continu, décodé côté PC.
   C'est le vrai mode d'usage (fluide, faible débit).

## 4. Injection des gestes et du clavier

- **Incrément 1** : `input tap x y` (root, zéro mise en place) — suffisant pour
  valider la boucle clic → toucher.
- **Vrai mode** : un périphérique virtuel persistant via `/dev/uinput` (on crée une
  fois un écran tactile + un clavier, puis on écrit les événements). Beaucoup plus
  faible latence, multitouch, scroll, touches — c'est l'amélioration prévue dès
  l'incrément 1 validé.

## 5. Sécurité

- Jeton aléatoire 32 hex (`/dev/urandom`, comme `p900ctl`), **régénéré à chaque
  démarrage**, affiché sur le téléphone, exigé dans la poignée de main. Connexion
  sans jeton valide = fermée et journalisée.
- Socket liée à la **seule interface USB** + IP tether. Aucune exposition ailleurs.
- **Désactivé par défaut** : activé seulement depuis la section dédiée de l'appli,
  le temps d'une session.
- **Journal des connexions** `/data/local/soc/screenctl.log` (horodatage, pair,
  événement, octets) → ingéré par l'agent Wazuh du téléphone, cohérent avec la
  traçabilité SOC du projet.

Posture identique à la note de sécurité V3 du README PurpleOps : un service qui
injecte des entrées = pouvoir fort, donc liaison d'interface + jeton + défaut-off
+ journal, non négociables.

## 6. Risques

- Trou d'image à la relance `screenrecord` (3 min) — masqué par relance anticipée.
- Surfaces sécurisées noires — accepté.
- Mapping des coordonnées sous rotation / mise à l'échelle de la fenêtre PC — géré
  côté client (remise à l'échelle inverse).
- Droits `/dev/uinput` — service root, OK.
- Jeton visible à l'écran (shoulder-surfing) — risque local seulement, lien câble.

## 7. Plan d'incréments (chacun mesuré : latence + img/s)

| # | Contenu | Capture | Entrée | Mesure |
| --- | --- | --- | --- | --- |
| **1** | image fixe + toucher | `screencap` brut | `input tap` | latence aller-retour, img/s en tirage continu |
| 2a | flux MJPEG | `screencap` boucle + JPEG | uinput | img/s, débit |
| 2b | flux H.264 | `screenrecord -` | uinput | latence, img/s, débit |
| 3 | presse-papiers + fichiers | — | — | — |

Le présent dépôt livre l'**incrément 1**.
