#!/usr/bin/env bash
# Applique l'overlay "commande série" sur un checkout de risinek/esp32-wifi-penetration-tool.
# Usage: apply_overlay.sh <chemin_du_checkout_risinek>
set -euo pipefail

FW="${1:?usage: apply_overlay.sh <fw_dir>}"
HERE="$(cd "$(dirname "$0")" && pwd)"

# 1) deposer le composant dans main/
cp "$HERE/cmd_serial.c" "$HERE/cmd_serial.h" "$FW/main/"

# 2) ajouter cmd_serial.c aux sources
sed -i 's#idf_component_register(SRCS "attack_dos.c"#idf_component_register(SRCS "cmd_serial.c" "attack_dos.c"#' "$FW/main/CMakeLists.txt"

# 3) patcher main.c : include + appel apres webserver_run()
sed -i 's@#include "webserver.h"@#include "webserver.h"\n#include "cmd_serial.h"@' "$FW/main/main.c"
sed -i 's@    webserver_run();@    webserver_run();\n    cmd_serial_start();@' "$FW/main/main.c"

echo "=== CMakeLists ==="; head -2 "$FW/main/CMakeLists.txt"
echo "=== main.c (verif) ==="; grep -nE 'cmd_serial|webserver_run' "$FW/main/main.c"
echo "overlay applique."
