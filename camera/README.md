# p900ctl — pilotage Wi-Fi d'une Nikon COOLPIX P900 (PTP/IP, Rust)

*English summary below.*

`p900ctl` parle directement à la P900 en PTP/IP (TCP 15740), sans l'appli Nikon.
Il fonctionne dans un chroot Linux sur Android, sous Termux, sur un PC Linux, ou
embarqué dans l'appli PurpleOps (section « Caméra Nikon »).

## État

| Élément | État |
| --- | --- |
| Client PTP/IP, téléchargement, propriétés, visée, déclenchement | écrit, testé contre le simulateur `fakecam` |
| Comportement de la vraie P900 | **non vérifié** : lancer `p900ctl probe` |
| Zoom à distance | hypothèse : propriété `FocalLength` (0x5008) ; à confirmer par `probe` / `watch` |

Mesuré sur une P900 : l'appareil est en **192.168.0.10**, le téléphone reçoit 192.168.0.11,
le Wi-Fi de l'appareil est ouvert (sans mot de passe).

## Utilisation

```
p900ctl probe --save rapport.txt   # ce que l'appareil accepte (lecture seule)
p900ctl props                      # réglages exposés
p900ctl ls --last 10               # dernières photos
p900ctl get --last 3 --out photos  # téléchargement
p900ctl capture --out photos       # déclenche et récupère la photo
p900ctl liveview --frames 50       # mesure la cadence de visée
p900ctl serve                      # visée + boutons sur http://127.0.0.1:8080/?t=<jeton>
```

Options utiles : `--host 192.168.0.10`, `--iface wlan0` (force le Wi-Fi, root requis),
`--trace` (journal des paquets).

## Construction

```
cargo test                                   # tests, dont client ↔ simulateur
cargo build --release -p p900ctl             # binaire natif (ex. dans le chroot arm64)
```

Pour Android sans chroot : cible `aarch64-linux-android` avec le NDK (voir la CI de PurpleOps).

## Sécurité

- `probe`, `props`, `ls`, `watch` ne modifient rien sur l'appareil.
- `serve` écoute sur 127.0.0.1 et exige un jeton aléatoire dans chaque requête.
- Le Wi-Fi de la P900 est ouvert : toute personne à portée peut s'y connecter
  et voir les échanges. À n'utiliser qu'avec ses propres appareils.

---

**English.** `p900ctl` talks PTP/IP (TCP 15740) to a Nikon COOLPIX P900 without
Nikon's app. Commands: `probe`, `props`, `ls`, `get`, `capture`, `liveview`, `zoom`,
`set-prop`, `watch`, `serve` (local web UI on 127.0.0.1 with a random token).
Tested against the bundled `fakecam` simulator; behaviour on a real P900 is not
yet verified. MIT licence.
