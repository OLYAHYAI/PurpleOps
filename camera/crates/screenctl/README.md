# screenctl — partage/pilotage de l'écran Android par câble USB (scrcpy maison)

Service **téléphone** de PurpleOps. Voir et piloter l'écran du Pixel 7a depuis le
PC Windows, **par câble USB-C uniquement**, sans `adb` et sans Wi-Fi. Pendant
`p900ctl` pour la Nikon : même style (Rust std pur, root KernelSU, jeton aléatoire).

Étude de conception (transport, choix de capture, codec, risques) : [`ETUDE.md`](ETUDE.md).
Client PC : [`../../pc-client/screenview`](../../pc-client/screenview).

## État : incrément 1 — image fixe + toucher

- Capture d'une trame via `screencap` (RGBA brut), envoyée au PC à la demande.
- Toucher, balayage, Retour/Accueil via `input`.
- Transport TCP lié à la **seule interface de partage USB**.
- Jeton 32 hex régénéré à chaque démarrage, exigé à la connexion.
- Journal `/data/local/soc/screenctl.log` (ingéré par l'agent Wazuh).

Suite prévue : uinput (latence/multitouch), flux MJPEG puis H.264 (`screenrecord`),
presse-papiers et fichiers. Voir le tableau d'incréments dans `ETUDE.md`.

## Mise en route

### Téléphone
Onglet **Matériel → Écran USB → ACTIVER** (désactivé par défaut). Le service, lancé
en root côté Android (pas dans Kali) :
1. passe la prise USB en carte réseau NCM (`svc usb setFunctions ncm`) ;
2. attend qu'Android donne une adresse à `ncm0` (le partage de connexion tire un
   sous-réseau au hasard, ex. 10.235.168.229/24, et fait le DHCP du PC) ;
3. écoute sur cette seule interface et affiche **adresse + jeton**.

**ARRÊTER** tue le service et remet la prise en mode normal (`screenctl usb-off`).

Prérequis matériel : câble **de données**, et le téléphone doit être
« contrôlé par l'appareil connecté » (Réglages → USB). Avec un câble C↔C, si le
téléphone se met en maître, utiliser un port USB-A du PC.

### PC (Windows 11, pilote NCM intégré, rien à installer)
`screenview.exe` est attaché à chaque Release GitHub. Double-clic ou :
```
screenview.exe <jeton>
```
L'adresse du téléphone est trouvée seule (passerelle de la carte « UsbNcm Host
Device »). Le jeton est demandé s'il manque.

| Souris | Action sur le téléphone |
| --- | --- |
| clic gauche | toucher |
| glisser (gauche) | balayage / défilement |
| clic droit | Retour |
| clic molette | Accueil |

Échap ferme la fenêtre. Le titre affiche img/s et latence.

### Mesure incrément 1 (Pixel 7a, 1080x2400, RGBA brut)
≈ 2,4 img/s, latence aller-retour ≈ 390 ms (dominée par `screencap` + 10 Mo/image).
Référence avant MJPEG / H.264.

## Sécurité

Service root qui capture l'écran et injecte des entrées : à traiter comme la note
V3 du README racine (RCE par conception). Garde-fous : socket liée au seul lien USB,
jeton obligatoire, **désactivé par défaut** (activé le temps d'une session), journal
systématique. À n'utiliser que sur son propre appareil.

## Construction

```sh
cargo test -p screenctl                      # tests hôte
# arm64 pour le téléphone : via la CI PurpleOps (NDK), cf. .github/workflows/android.yml
```
