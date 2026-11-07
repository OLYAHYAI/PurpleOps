# screenctl — partage/pilotage de l'écran Android par câble USB (scrcpy maison)

Service **téléphone** de PurpleOps. Voir et piloter l'écran du Pixel 7a depuis le
PC Windows, **par câble USB-C uniquement**, sans `adb` et sans Wi-Fi. Pendant
`p900ctl` pour la Nikon : même style (Rust std pur, root KernelSU, jeton aléatoire).

Étude de conception (transport, choix de capture, codec, risques) : [`ETUDE.md`](ETUDE.md).
Client PC : [`../../pc-client/screenview`](../../pc-client/screenview).

## État : incrément 1 — image fixe + toucher

- Capture d'une trame via `screencap` (RGBA brut), envoyée au PC à la demande.
- Toucher injecté via `input tap x y`.
- Transport TCP lié à la **seule interface de partage USB**.
- Jeton 32 hex régénéré à chaque démarrage, exigé à la connexion.
- Journal `/data/local/soc/screenctl.log` (ingéré par l'agent Wazuh).

Suite prévue : uinput (latence/multitouch), flux MJPEG puis H.264 (`screenrecord`),
presse-papiers et fichiers. Voir le tableau d'incréments dans `ETUDE.md`.

## Mise en route (test de l'incrément 1)

### 1. Lien USB
Sur le téléphone : **Paramètres → Partage de connexion → Partage par USB** (ON).
Le téléphone prend l'adresse **192.168.42.129** sur l'interface `rndis0` (ou `ncm0`).
Vérifier le nom exact : `ip -4 addr | grep -E 'rndis|ncm'`.

### 2. Démarrer le service (root), comme `p900ctl`
Le binaire est embarqué dans l'APK sous `libscreenctl.so`. Lancement root :

```sh
BIN=$(dirname "$(pm path com.jbr.purpleops | head -1 | sed 's/package://')")/lib/arm64/libscreenctl.so
# pm path renvoie .../base.apk ; les .so sont extraits à côté, dans lib/arm64/
su -c "exec '$BIN' serve --iface rndis0 --listen 192.168.42.129:8099"
```

Le service affiche :
```
SCREENCTL token=<32 hex> listen=192.168.42.129:8099 iface=rndis0
Jeton de session : <32 hex>
```
Note le jeton.

### 3. Client PC (Windows)
```
cd camera\pc-client\screenview
cargo run --release -- 192.168.42.129 8099 <le jeton>
```
Une fenêtre affiche l'écran du téléphone. **Clic gauche = toucher** au point
correspondant. Le titre et la console affichent **img/s** et **latence aller-retour**.
Échap pour quitter.

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
