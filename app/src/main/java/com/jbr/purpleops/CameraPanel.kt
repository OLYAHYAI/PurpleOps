package com.jbr.purpleops

import android.annotation.SuppressLint
import android.view.View
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import java.io.File

/**
 * Section « Caméra Nikon » : pilote une COOLPIX P900 en Wi-Fi (PTP/IP) via le
 * binaire Rust p900ctl embarqué dans l'APK (lib/arm64/libp900ctl.so).
 *
 * - « Tester » lance `p900ctl probe` en root et affiche ce que l'appareil accepte.
 * - « Visée » lance `p900ctl serve` en root sur 127.0.0.1:8080 et affiche son
 *   interface (visée, déclenchement, zoom) dans une WebView. Le jeton aléatoire
 *   imprimé par p900ctl est lu sur sa sortie et passé dans l'URL.
 *
 * Les sockets sont liées à wlan0 (--iface) : le trafic part par le Wi-Fi de
 * l'appareil photo même si les données mobiles sont actives.
 */
class CameraPanel(
    private val act: AppCompatActivity,
    root: View,
    private val scope: CoroutineScope,
) {
    private val status: TextView = root.findViewById(R.id.camStatus)
    private val probeBtn: TextView = root.findViewById(R.id.camProbe)
    private val startBtn: TextView = root.findViewById(R.id.camStart)
    private val stopBtn: TextView = root.findViewById(R.id.camStop)
    private val web: WebView = root.findViewById(R.id.camWeb)
    private val log: TextView = root.findViewById(R.id.camLog)
    private val logScroll: ScrollView = root.findViewById(R.id.camLogScroll)

    @Volatile
    private var serveProc: Process? = null

    private val bin: String get() = act.applicationInfo.nativeLibraryDir + "/libp900ctl.so"
    private val home = "/data/local/tmp/p900ctl"
    private val outDir = "/sdcard/DCIM/P900"
    private val urlRegex = Regex("http://127\\.0\\.0\\.1:8080/\\?t=[0-9a-f]{32}")

    init {
        setupWeb()
        probeBtn.setOnClickListener { probe() }
        startBtn.setOnClickListener { start() }
        stopBtn.setOnClickListener { stop(); status.text = "Arrêt demandé" }
        status.text = if (File(bin).exists()) "Prêt — connectez le téléphone au Wi-Fi « Nikon… » de l'appareil"
        else "p900ctl absent de cette version de l'APK"
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWeb() {
        web.settings.javaScriptEnabled = true
        web.settings.allowFileAccess = false
        web.settings.allowContentAccess = false
        web.webViewClient = object : WebViewClient() {
            // Seule l'interface locale de p900ctl peut s'afficher ici.
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
                request.url.host != "127.0.0.1"
        }
    }

    private fun append(s: String) {
        log.append(s + "\n")
        logScroll.post { logScroll.fullScroll(View.FOCUS_DOWN) }
    }

    private fun ready(): Boolean {
        if (!File(bin).exists()) {
            status.text = "p900ctl absent de cette version de l'APK"
            return false
        }
        return true
    }

    private fun probe() {
        if (!ready()) return
        status.text = "Relevé en cours…"
        scope.launch {
            val cmd = "mkdir -p $home && HOME=$home '$bin' --iface wlan0 --timeout 6 probe 2>&1"
            val (code, out) = RootRunner.runAndroidRoot(cmd)
            append(out.trim())
            status.text = if (code == 0) "Relevé terminé — détails dans le journal" else "Échec du relevé (code $code)"
        }
    }

    private fun start() {
        if (!ready() || serveProc != null) return
        status.text = "Connexion à l'appareil…"
        scope.launch(Dispatchers.IO) {
            try {
                val cmd = "mkdir -p $home $outDir && HOME=$home exec '$bin' --iface wlan0 " +
                    "serve --listen 127.0.0.1:8080 --out $outDir 2>&1"
                val p = ProcessBuilder("su", "-c", cmd).redirectErrorStream(true).start()
                serveProc = p
                p.inputStream.bufferedReader().forEachLine { line ->
                    val url = urlRegex.find(line)?.value
                    act.runOnUiThread {
                        append(line)
                        if (url != null) {
                            web.visibility = View.VISIBLE
                            web.loadUrl(url)
                            status.text = "Visée active — photos dans $outDir"
                        }
                    }
                }
                val code = p.waitFor()
                act.runOnUiThread {
                    status.text = "Service caméra arrêté (code $code)"
                    web.loadUrl("about:blank")
                    web.visibility = View.GONE
                }
            } catch (e: Exception) {
                act.runOnUiThread { append("[erreur] ${e.message}") }
            } finally {
                serveProc = null
            }
        }
    }

    /** Arrête le service ; le processus root peut survivre à `su`, d'où le pkill. */
    fun stop() {
        serveProc?.destroy()
        scope.launch { RootRunner.runAndroidRoot("pkill -f libp900ctl.so") }
    }
}
