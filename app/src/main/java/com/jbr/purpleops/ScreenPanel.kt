package com.jbr.purpleops

import android.view.View
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.io.File

/**
 * Section « Écran USB » : partage et pilotage de l'écran depuis le PC par le câble
 * USB (scrcpy maison), via le binaire Rust screenctl embarqué dans l'APK
 * (lib/arm64/libscreenctl.so).
 *
 * Désactivé par défaut. « ACTIVER » lance, en root côté Android (pas dans Kali) :
 *   screenctl serve --usb   -> prise USB en carte réseau NCM, attente de l'adresse,
 *                              écoute sur cette seule interface, jeton aléatoire.
 * La section affiche l'adresse et le jeton à saisir dans screenview.exe.
 * « ARRÊTER » tue le service et remet la prise USB en mode normal.
 */
class ScreenPanel(
    private val act: AppCompatActivity,
    root: View,
    private val scope: CoroutineScope,
) {
    private val status: TextView = root.findViewById(R.id.scrStatus)
    private val toggle: TextView = root.findViewById(R.id.scrToggle)
    private val info: TextView = root.findViewById(R.id.scrInfo)
    private val log: TextView = root.findViewById(R.id.camLog)
    private val logScroll: ScrollView = root.findViewById(R.id.camLogScroll)

    @Volatile
    private var proc: Process? = null

    private val bin: String get() = act.applicationInfo.nativeLibraryDir + "/libscreenctl.so"
    private val readyRegex = Regex("SCREENCTL token=([0-9a-f]{32}) listen=([0-9.]+):(\\d+)")

    init {
        toggle.setOnClickListener { if (proc == null) start() else stop() }
        idle(if (File(bin).exists()) "Désactivé — branche le câble au PC puis ACTIVER"
              else "screenctl absent de cette version de l'APK")
    }

    private fun idle(msg: String) {
        status.text = msg
        toggle.text = "ACTIVER"
        info.visibility = View.GONE
    }

    private fun append(s: String) {
        log.append("[écran] $s\n")
        logScroll.post { logScroll.fullScroll(View.FOCUS_DOWN) }
    }

    private fun start() {
        if (!File(bin).exists() || proc != null) return
        status.text = "Montage du lien USB…"
        toggle.text = "ARRÊTER"
        scope.launch(Dispatchers.IO) {
            try {
                val p = ProcessBuilder("su", "-c", "exec '$bin' serve --usb 2>&1")
                    .redirectErrorStream(true).start()
                proc = p
                p.inputStream.bufferedReader().forEachLine { line ->
                    val m = readyRegex.find(line)
                    act.runOnUiThread {
                        append(line)
                        if (m != null) {
                            val (token, ip, port) = m.destructured
                            status.text = "Actif — en attente du PC"
                            info.text = "Adresse : $ip   port $port\nJeton : $token\n\n" +
                                "PC : screenview.exe $token"
                            info.visibility = View.VISIBLE
                        } else if (line.contains(" auth ") && line.endsWith(" ok")) {
                            status.text = "PC connecté"
                        }
                    }
                }
                val code = p.waitFor()
                act.runOnUiThread { idle("Service arrêté (code $code)") }
            } catch (e: Exception) {
                act.runOnUiThread { append("[erreur] ${e.message}"); idle("Erreur au démarrage") }
            } finally {
                proc = null
            }
        }
    }

    /** Arrête le service ; le processus root peut survivre à `su`, d'où le pkill. */
    fun stop() {
        // Jamais lancé : on ne touche pas au mode USB (ex. transfert de fichiers en cours).
        val p = proc ?: return
        p.destroy()
        scope.launch {
            RootRunner.runAndroidRoot("pkill -f libscreenctl.so; '$bin' usb-off")
            idle("Désactivé — prise USB remise en mode normal")
        }
    }
}
