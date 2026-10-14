#!/bin/sh
# Mise à jour À LA DEMANDE du serveur PurpleOps MCP, DANS le chroot Debian.
# Récupère server.py depuis le repo public, SANS toucher config.yaml ni le token.
# À lancer TOI-MÊME dans le chroot Debian. Ne jamais mettre `exit` à la suite.
set -eu

RAW="https://raw.githubusercontent.com/OLYAHYAI/PurpleOps/main/mcp-server"
DEST="/opt/purpleops-mcp"
# Le chroot peut hériter d'un TMPDIR Termux inexistant -> on force /tmp.
TMPDIR=/tmp; export TMPDIR; mkdir -p /tmp
TMP="$(mktemp -d /tmp/po.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

echo "[maj] téléchargement depuis $RAW"
for f in server.py requirements.txt kali-mcp-start.sh; do
  curl -fsSL "$RAW/$f" -o "$TMP/$f"
done

# Garde-fou : on ne remplace PAS un serveur sain par un fetch cassé.
python3 -m py_compile "$TMP/server.py"

mkdir -p "$DEST"
# config.yaml : créé depuis l'exemple SEULEMENT s'il est absent ; jamais écrasé.
if [ ! -f "$DEST/config.yaml" ]; then
  curl -fsSL "$RAW/config.example.yaml" -o "$DEST/config.yaml"
  echo "[init] $DEST/config.yaml créé depuis l'exemple — édite IP/chemins/kali_invoke."
fi

cp "$TMP/server.py"        "$DEST/server.py"
cp "$TMP/requirements.txt" "$DEST/requirements.txt"
cp "$TMP/kali-mcp-start.sh" "$DEST/kali-mcp-start.sh"
chmod +x "$DEST/kali-mcp-start.sh"

echo "[ok] serveur à jour dans $DEST (config.yaml et token préservés)."
echo "     Dépendances : pip3 install -r $DEST/requirements.txt  (si première fois / changement)"
