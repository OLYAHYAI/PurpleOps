package com.jbr.purpleops

import android.content.Context
import android.view.View
import android.widget.CheckBox
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.io.File

/**
 * Onglet « Écran » : écran du téléphone vu et piloté depuis le PC par le câble USB
 * (scrcpy maison), via le binaire Rust screenctl embarqué (lib/arm64/libscreenctl.so),
 * lancé en root côté Android (pas dans Kali).
 *
 * - ACTIVER : `screenctl serve --usb` (prise USB en carte réseau, réarmement auto).
 * - APPAIRER UN PC : ouvre 2 min pendant lesquelles un PC inconnu est accepté et
 *   mémorisé ; ensuite un double-clic sur screenview.exe suffit, sans jeton.
 * - Démarrage automatique (décoché par défaut) : le service démarre avec l'appli et
 *   continue de tourner quand on la ferme.
 */
class ScreenPanel(
    private val act: AppCompatActivity,
    root: View,
    private val scope: CoroutineScope,
) {
    private val status: TextView = root.findViewById(R.id.scrStatus)
    private val toggle: TextView = root.findViewById(R.id.scrToggle)
    private val info: TextView = root.findViewById(R.id.scrInfo)
    private val pairBtn: TextView = root.findViewById(R.id.scrPair)
    private val forgetBtn: TextView = root.findViewById(R.id.scrForget)
    private val auto: CheckBox = root.findViewById(R.id.scrAuto)
    private val pcs: TextView = root.findViewById(R.id.scrPcs)
    private val log: TextView = root.findViewById(R.id.scrLog)
    private val logScroll: ScrollView = root.findViewById(R.id.scrLogScroll)

    private val prefs = act.getSharedPreferences("purpleops", Context.MODE_PRIVATE)
    private val bin: String get() = act.applicationInfo.nativeLibraryDir + "/libscreenctl.so"
    private val pcsFile = "/data/local/soc/screenctl_pcs"

    @Volatile
    private var proc: Process? = null

    /** Service en marche, lancé par nous ou par une instance précédente de l'appli. */
    @Volatile
    private var running = false

    // pidof (nom du programme) et non pkill -f : la ligne de commande du shell qui
    // lance pkill contient aussi « libscreenctl.so », il se tuerait lui-même.
    private val KILL = "kill \$(pidof libscreenctl.so) 2>/dev/null"

    private val ev = Regex("^SCREENCTL (\\S+)(?: (.*))?$")

    init {
        auto.isChecked = prefs.getBoolean("screen_auto", false)
        auto.setOnCheckedChangeListener { _, on -> prefs.edit().putBoolean("screen_auto", on).apply() }
        toggle.setOnClickListener { if (running) stop() else start() }
        pairBtn.setOnClickListener { pair() }
        forgetBtn.setOnClickListener { forget() }

        if (!File(bin).exists()) {
            setIdle("screenctl absent de cette version de l'APK")
        } else {
            setIdle("Désactivé")
            scope.launch {
                val (code, _) = RootRunner.runAndroidRoot("pidof libscreenctl.so")
                if (code == 0) {
                    running = true
                    toggle.text = "ARRÊTER"
                    status.text = "Actif (lancé précédemment)"
                    info.text = "Sur le PC : double-clic sur screenview.exe."
                } else if (auto.isChecked) {
                    start()
                }
                refreshPcs()
            }
        }
    }

    private fun setIdle(msg: String) {
        running = false
        status.text = msg
        toggle.text = "ACTIVER"
        info.text = "Branche le câble au PC puis ACTIVER.\nSur le PC : double-clic sur screenview.exe (Release GitHub)."
    }

    private fun append(s: String) {
        log.append(s + "\n")
        logScroll.post { logScroll.fullScroll(View.FOCUS_DOWN) }
    }

    private fun start() {
        if (!File(bin).exists() || running) return
        running = true
        toggle.text = "ARRÊTER"
        status.text = "Démarrage…"
        scope.launch(Dispatchers.IO) {
            try {
                val p = ProcessBuilder("su", "-c", "exec '$bin' serve --usb 2>&1")
                    .redirectErrorStream(true).start()
                proc = p
                p.inputStream.bufferedReader().forEachLine { line ->
                    act.runOnUiThread { onLine(line) }
                }
                val code = p.waitFor()
                act.runOnUiThread { setIdle("Service arrêté (code $code)") }
            } catch (e: Exception) {
                act.runOnUiThread { append("[erreur] ${e.message}"); setIdle("Erreur au démarrage") }
            } finally {
                proc = null
            }
        }
    }

    private fun onLine(line: String) {
        val m = ev.find(line)
        if (m == null) {
            append(line)
            return
        }
        val (what, rest) = m.destructured
        val host = rest.substringAfter("host=", "")
        when (what) {
            "listen" -> {
                val ip = rest.substringAfter("listen=").substringBefore(':')
                status.text = "Actif — en attente du PC"
                info.text = "Téléphone : $ip\nSur le PC : double-clic sur screenview.exe."
            }
            "waiting-cable" -> status.text = "En attente du câble USB (branché au PC)…"
            "link-lost" -> status.text = "Câble débranché — reprise automatique au rebranchement"
            "connected" -> status.text = "PC connecté : $host"
            "disconnected" -> status.text = "Actif — $host déconnecté"
            "paired" -> {
                Toast.makeText(act, "PC « $host » appairé", Toast.LENGTH_SHORT).show()
                refreshPcs()
            }
            "refused" -> status.text = "PC « $host » refusé : appuie sur APPAIRER UN PC"
        }
        append(line.removePrefix("SCREENCTL "))
    }

    private fun pair() {
        if (!running) start()
        scope.launch {
            val (code, out) = RootRunner.runAndroidRoot("'$bin' pair 120")
            if (code == 0) {
                status.text = "Appairage ouvert 2 min : lance screenview.exe sur le PC"
            } else {
                append("[appairage] ${out.trim()}")
            }
        }
    }

    private fun forget() {
        AlertDialog.Builder(act)
            .setTitle("Oublier tous les PC ?")
            .setMessage("Chaque PC devra être appairé à nouveau.")
            .setPositiveButton("Oublier") { _, _ ->
                scope.launch {
                    RootRunner.runAndroidRoot("'$bin' forget")
                    refreshPcs()
                }
            }
            .setNegativeButton("Annuler", null)
            .show()
    }

    fun refreshPcs() {
        scope.launch {
            val (_, out) = RootRunner.runAndroidRoot("cat $pcsFile 2>/dev/null")
            val names = out.lines().mapNotNull { it.trim().split(Regex("\\s+")).getOrNull(1) }.distinct()
            pcs.text = if (names.isEmpty()) "PC appairés : aucun" else "PC appairés : " + names.joinToString(", ")
        }
    }

    /** ARRÊTER : coupe le service (où qu'il ait été lancé) et rend la prise USB. */
    fun stop() {
        proc?.destroy()
        running = false
        scope.launch {
            RootRunner.runAndroidRoot(KILL + "; '$bin' usb-off")
            setIdle("Désactivé — prise USB remise en mode normal")
        }
    }

    /** Fermeture de l'appli : en démarrage automatique, le service continue de tourner. */
    fun onAppDestroyed() {
        if (running && !auto.isChecked) {
            proc?.destroy()
            scope.launch { RootRunner.runAndroidRoot(KILL + "; '$bin' usb-off") }
        }
    }
}
