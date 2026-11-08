package com.jbr.purpleops

import android.content.Context
import android.view.View
import android.widget.CheckBox
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Réglages → « Serveur MCP PurpleOps » : démarre le serveur MCP (mcp-server/server.py,
 * pour Claude Desktop via Tailscale) sans taper de commande.
 *
 * - Le serveur est cherché dans les deux chroots (/data/local/kali puis
 *   /data/local/debian, dossier /opt/purpleops-mcp) : la section affiche où il est
 *   installé, ce qui tranche la question « Kali ou Debian ».
 * - Démarrage : script de montage du chroot (kali-start.sh / debian-start.sh), puis
 *   kali-mcp-start.sh dans le chroot, détaché (setsid) et sorti du cgroup de l'appli
 *   pour survivre à sa fermeture. Sortie : <chroot>/var/log/purpleops-mcp.out.
 * - Démarrage automatique à l'ouverture de l'appli : case cochée par défaut (choix
 *   de l'utilisateur), décochable. Les barrières du serveur restent les mêmes :
 *   bind Tailscale uniquement, jeton Bearer, journal d'audit lu par Wazuh.
 */
class McpPanel(
    private val act: AppCompatActivity,
    root: View,
    private val scope: CoroutineScope,
) {
    private val status: TextView = root.findViewById(R.id.mcpStatus)
    private val toggle: TextView = root.findViewById(R.id.mcpToggle)
    private val auto: CheckBox = root.findViewById(R.id.mcpAuto)
    private val updateBtn: TextView = root.findViewById(R.id.mcpUpdate)
    private val prefs = act.getSharedPreferences("purpleops", Context.MODE_PRIVATE)

    // « Actif » = quelque chose ÉCOUTE sur le port 8765 (0x223D), lu dans la table
    // réseau du noyau. L'ancienne détection (pgrep -f "purpleops-mc[p]/server.py")
    // se trouvait elle-même dans la ligne de commande du shell (qui contenait aussi
    // « …purpleops-mcp/server.py » via FIND) et ne correspondait pas au vrai
    // processus (« venv/bin/python server.py ») : faux « Actif ».
    private val LISTEN = "cat /proc/net/tcp /proc/net/tcp6 2>/dev/null | grep -qiE ':223D [0-9A-F:]+ 0A' && echo LISTEN"

    /** Processus dont le dossier courant est celui du serveur (lancé par kali-mcp-start.sh). */
    private fun pids(chroot: String) = "for p in /proc/[0-9]*; do " +
        "[ \"\$(readlink \$p/cwd 2>/dev/null)\" = '$chroot/opt/purpleops-mcp' ] && echo \${p#/proc/}; done"

    private val FIND = "for R in /data/local/kali /data/local/debian; do " +
        "[ -f \$R/opt/purpleops-mcp/server.py ] && echo \$R && break; done"

    @Volatile
    private var running = false

    init {
        auto.isChecked = prefs.getBoolean("mcp_auto", true)
        auto.setOnCheckedChangeListener { _, on -> prefs.edit().putBoolean("mcp_auto", on).apply() }
        toggle.setOnClickListener { if (running) stop() else start() }
        updateBtn.setOnClickListener { update() }
        scope.launch {
            refresh()
            if (!running && auto.isChecked) start()
        }
    }

    /** Où est installé le serveur, et tourne-t-il ? */
    private suspend fun refresh(): String? {
        val (_, out) = RootRunner.runAndroidRoot("$FIND; $LISTEN")
        val lines = out.lines().map { it.trim() }.filter { it.isNotEmpty() }
        val chroot = lines.firstOrNull { it.startsWith("/data/local/") }
        running = "LISTEN" in lines
        val where = chroot?.substringAfterLast('/') ?: "?"
        when {
            chroot == null -> {
                status.text = "Serveur MCP introuvable (ni /data/local/kali ni /data/local/debian → /opt/purpleops-mcp)"
                toggle.isEnabled = false
            }
            running -> status.text = "Actif — chroot $where, écoute sur le port 8765 (IP Tailscale)"
            else -> {
                val (_, tail) = RootRunner.runAndroidRoot("tail -n 4 $chroot/var/log/purpleops-mcp.out 2>/dev/null")
                status.text = "Arrêté — chroot $where" +
                    (if (tail.isNotBlank()) "\nDernière sortie : " + tail.trim().lines().takeLast(3).joinToString(" / ") else "")
            }
        }
        toggle.text = if (running) "ARRÊTER" else "DÉMARRER"
        return chroot
    }

    fun start() {
        scope.launch {
            val chroot = refresh() ?: return@launch
            if (running) return@launch
            status.text = "Démarrage du serveur MCP…"
            val name = chroot.substringAfterLast('/')
            val out = "$chroot/var/log/purpleops-mcp.out"
            val inner = "env -i HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin " +
                "chroot $chroot /bin/sh /opt/purpleops-mcp/kali-mcp-start.sh"
            val cmd = "/data/local/$name-start.sh >/dev/null 2>&1; " +
                "sh -c 'echo \$\$ > /sys/fs/cgroup/cgroup.procs 2>/dev/null; " +
                "exec setsid $inner >> $out 2>&1 < /dev/null' &"
            RootRunner.runAndroidRoot(cmd)
            delay(4000) // laisse le temps à uvicorn de démarrer (ou au script d'échouer)
            refresh()
            if (!running) {
                val (_, tail) = RootRunner.runAndroidRoot("tail -n 3 $out 2>/dev/null")
                status.text = "Échec du démarrage (chroot $name) : " + tail.trim().ifEmpty { "aucune sortie" }
            }
        }
    }

    /**
     * Récupère le dernier server.py depuis GitHub avec purpleops-mcp-update.sh (qui
     * conserve config.yaml et le jeton, et vérifie la syntaxe avant de remplacer),
     * puis redémarre le serveur. Le code du MCP vit dans le chroot : il ne suit pas
     * les mises à jour de l'APK.
     */
    private fun update() {
        scope.launch {
            val chroot = refresh() ?: return@launch
            val name = chroot.substringAfterLast('/')
            status.text = "Mise à jour du serveur MCP…"
            val raw = "https://raw.githubusercontent.com/OLYAHYAI/PurpleOps/main/mcp-server/purpleops-mcp-update.sh"
            val (code, out) = RootRunner.runAndroidRoot(
                "/data/local/$name-start.sh >/dev/null 2>&1; " +
                    "env -i HOME=/root PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin " +
                    "chroot $chroot /bin/sh -c 'curl -fsSL $raw -o /tmp/po-mcp-update.sh && sh /tmp/po-mcp-update.sh' 2>&1"
            )
            if (code != 0) {
                status.text = "Échec de la mise à jour : " + out.trim().lines().takeLast(2).joinToString(" ")
                return@launch
            }
            if (running) {
                RootRunner.runAndroidRoot("for i in \$(${pids(chroot)}); do kill \$i; done")
                delay(800)
                running = false
            }
            start()
        }
    }

    fun stop() {
        scope.launch {
            val (_, out) = RootRunner.runAndroidRoot(FIND)
            val chroot = out.lines().map { it.trim() }.firstOrNull { it.startsWith("/data/local/") } ?: return@launch
            RootRunner.runAndroidRoot("for i in \$(${pids(chroot)}); do kill \$i; done")
            delay(800)
            refresh()
        }
    }
}
