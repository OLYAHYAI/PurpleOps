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
    private val prefs = act.getSharedPreferences("purpleops", Context.MODE_PRIVATE)

    // Crochets : le motif ne correspond pas à la ligne de commande de pgrep/pkill
    // eux-mêmes (qui contient « purpleops-mc[p] », pas « purpleops-mcp »).
    private val PATTERN = "purpleops-mc[p]/server.py"
    private val FIND = "for R in /data/local/kali /data/local/debian; do " +
        "[ -f \$R/opt/purpleops-mcp/server.py ] && echo \$R && break; done"

    @Volatile
    private var running = false

    init {
        auto.isChecked = prefs.getBoolean("mcp_auto", true)
        auto.setOnCheckedChangeListener { _, on -> prefs.edit().putBoolean("mcp_auto", on).apply() }
        toggle.setOnClickListener { if (running) stop() else start() }
        scope.launch {
            refresh()
            if (!running && auto.isChecked) start()
        }
    }

    /** Où est installé le serveur, et tourne-t-il ? */
    private suspend fun refresh(): String? {
        val (_, out) = RootRunner.runAndroidRoot("$FIND; pgrep -f '$PATTERN' >/dev/null && echo RUNNING")
        val lines = out.lines().map { it.trim() }.filter { it.isNotEmpty() }
        val chroot = lines.firstOrNull { it.startsWith("/data/local/") }
        running = "RUNNING" in lines
        val where = chroot?.substringAfterLast('/') ?: "?"
        when {
            chroot == null -> {
                status.text = "Serveur MCP introuvable (ni /data/local/kali ni /data/local/debian → /opt/purpleops-mcp)"
                toggle.isEnabled = false
            }
            running -> status.text = "Actif — chroot $where, port 8765 sur l'IP Tailscale"
            else -> status.text = "Arrêté — installé dans le chroot $where"
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

    fun stop() {
        scope.launch {
            RootRunner.runAndroidRoot("pkill -f '$PATTERN'")
            delay(800)
            refresh()
        }
    }
}
