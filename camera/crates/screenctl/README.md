# screenctl — écran Android piloté depuis le PC par le câble USB (scrcpy maison)

Service **téléphone** de PurpleOps (Rust std pur, root KernelSU), client PC
[`screenview`](../../pc-client/screenview) (Windows). Étude de conception : [`ETUDE.md`](ETUDE.md).

## Usage quotidien

**Une seule fois** : PurpleOps → onglet **Écran** → **ACTIVER** → **APPAIRER UN PC**,
puis double-clic sur `screenview.exe` (Release GitHub) dans les 2 minutes.
Le PC est mémorisé.

**Ensuite** : double-clic sur `screenview.exe`. Rien à saisir : le téléphone est
trouvé seul (passerelle de la carte « UsbNcm »), le PC s'identifie par sa clé
(`%APPDATA%\PurpleOps\screenview.key`). L'exe peut rester ouvert : il attend le
téléphone et se reconnecte seul.

Case **Démarrer automatiquement** (décochée par défaut) : le service démarre avec
l'appli et continue de tourner quand on la ferme. Débrancher pour sortir avec la
caméra puis rebrancher : le lien se réarme tout seul.

| PC | Téléphone |
| --- | --- |
| clic gauche (maintenu, glissé) | doigt : toucher, glisser, appui long, défilement avec inertie |
| molette | défiler |
| clic droit / Échap | Retour |
| clic molette | Accueil |
| clavier | texte, Entrée, Effacer, flèches, Ctrl+A/C/V/X/Z |

## Architecture

| Élément | Choix |
| --- | --- |
| Lien | prise USB en carte réseau NCM (`svc usb setFunctions ncm`), écoute sur `ncm0` seul ; Android choisit le sous-réseau et fait le DHCP du PC |
| Vidéo | H.264 matériel (`screenrecord`), **délimité image par image côté téléphone** (u32 taille + image) : le PC ne devine jamais où une image finit |
| Entrées | écran tactile multipoint + clavier virtuels **/dev/uinput** : ~0,1 ms par événement (contre ~500 ms avec la commande `input`) |
| Accès | appairage façon ADB : clé du PC, acceptée seulement pendant la fenêtre ouverte depuis le téléphone (2 min) ; liste `/data/local/soc/screenctl_pcs` (0600) |
| Journal SOC | `/data/local/soc/screenctl.log` : démarrage, appairages, refus, sessions |

Commandes : `screenctl serve --usb` · `pair [s]` · `forget` · `usb-off`.

## Mesures

| Version | Cadence | Latence / remarques |
| --- | --- | --- |
| Incr. 1 : image brute `screencap` | 2,4 img/s | ~390 ms (10,4 Mo/image) |
| Incr. 2 : H.264 | jusqu'à **60 img/s**, 3–10 Mbit/s | décodage PC ~11 ms ; pics dus à la délimitation par silence (corrigé) et aux gestes `input` (remplacés par uinput) |

`screenrecord` n'envoie une image que si l'écran change : écran immobile = 0 img/s.

## Limites connues

- Texte : caractères ASCII seulement pour l'instant (disposition US côté téléphone) ;
  accents et presse-papiers PC ↔ téléphone : prochaine étape (aide Java via `app_process`).
- Rotation de l'écran : figée au début de chaque segment `screenrecord` (3 min).

## Sécurité

Service root qui capture l'écran et injecte des entrées : liaison au seul lien USB,
appairage explicite sur le téléphone, désactivé par défaut, journal systématique.
À n'utiliser que sur son propre appareil.
