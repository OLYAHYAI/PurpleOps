#!/usr/bin/env bash
# PurpleOps - Lab Arsenal installer (chroot Kali, Pixel 7a)
# PERIMETRE : ton homelab PFE SOAR uniquement. Ne jamais lancer hors lab.
#
# Usage (dans le chroot Kali, en root) :
#   ./install-lab-arsenal.sh           # outils on-path / MITM (utilisables DEPUIS le tel)
#   ./install-lab-arsenal.sh --wifi    # suite WiFi monitor (POUR capteur Pi/laptop, PAS le tel)
#   ./install-lab-arsenal.sh --all     # les deux
set -euo pipefail

ONPATH=1; WIFI=0
for a in "$@"; do case "$a" in
  --wifi)   ONPATH=0; WIFI=1;;
  --all)    ONPATH=1; WIFI=1;;
  --onpath) ONPATH=1;;
  *) echo "arg inconnu: $a (attendus: --wifi | --all | --onpath)";;
esac; done

echo "== PurpleOps lab-arsenal =="
[ "$(id -u)" -eq 0 ] || { echo "Lance en root (su / chroot)"; exit 1; }
export DEBIAN_FRONTEND=noninteractive
apt-get update -y

ONPATH_PKGS="bettercap ettercap-text-only dsniff python3-scapy mitmproxy tcpdump tshark arp-scan netdiscover macchanger iproute2 iptables nmap jq curl"
WIFI_PKGS="aircrack-ng hcxtools hcxdumptool kismet wifite iw wireless-tools"

if [ "$ONPATH" -eq 1 ]; then
  echo "-- Outils on-path / MITM (phone-usable) --"
  apt-get install -y $ONPATH_PKGS
fi
if [ "$WIFI" -eq 1 ]; then
  echo "-- Suite WiFi monitor (capteur Pi/laptop) --"
  echo "wireshark-common wireshark-common/install-setuid boolean true" | debconf-set-selections
  apt-get install -y $WIFI_PKGS
fi

echo ""
echo "== Sonde capacites (c'est le NOYAU qui decide, pas l'adaptateur) =="
echo "- Interfaces :"; ip -brief link 2>/dev/null || true
echo "- phy / modes supportes :"
(iw phy 2>/dev/null | grep -A8 "Supported interface modes") || echo "  iw: pas de phy expose dans le chroot (noyau stock probable)"
echo "  >>> cherche le mot 'monitor' ci-dessus. Absent = pas de mode moniteur sur le tel."
echo "- NET_RAW (raw socket pour ARP-spoof / injection) :"
python3 - <<'PY' 2>/dev/null || echo "  test raw socket KO"
import socket
try:
    s=socket.socket(socket.AF_PACKET, socket.SOCK_RAW); s.close()
    print("  OK: AF_PACKET raw dispo -> ARP-spoof / injection possibles depuis le tel")
except Exception as e:
    print("  KO:", e, "-> raw socket refuse dans ce chroot")
PY
echo ""
echo "Termine."
echo " * On-path (bettercap/ettercap/dsniff) : marche sur wlan0 en mode GERE, depuis le tel."
echo " * Monitor / airodump / kismet : NECESSITE noyau capable (NetHunter) ou capteur Pi/laptop."
