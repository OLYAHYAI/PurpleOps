package com.jbr.purpleops

import android.content.Context
import android.view.View
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Écran Radio (onglet Kali › Radio) : capteur WIDS wol-pi + ESP32.
 *
 * Refonte « Neon Ops » (19/11) : ce n'est plus une boîte de dialogue mais un
 * écran du même gabarit que Kali (layout `view_radio.xml`). L'URL du webhook
 * se règle dans Réglages. Les actions sensibles passent par [ConfirmSheet].
 *
 * Appelle le webhook n8n ([RadioClient]). Les actions offensives
 * (capture-handshake, deauth-test) restent verrouillées tant que le dernier
 * « Statut » rapporte `inject=none` (pas de carte AR9271) ; de toute façon le
 * Pi applique l'allowlist de BSSID et audite dans Wazuh (100733-100738).
 * Périmètre : TON réseau. Deauth ciblé par défaut (incident du 18/11).
 */
class RadioPanel(private val act: AppCompatActivity, root: View, private val ui: CoroutineScope) {

    private val ctx: Context get() = act.applicationContext
    private var injectAvailable = false
    private var statusLoaded = false
    private val hhmm = SimpleDateFormat("HH:mm", Locale.FRANCE)

    private val log: TextView = root.findViewById(R.id.radLog)
    private val chip: TextView = root.findViewById(R.id.radChip)
    private val last: TextView = root.findViewById(R.id.radLast)
    private val kismet: TextView = root.findViewById(R.id.radKismet)
    private val uplink: TextView = root.findViewById(R.id.radUplink)
    private val inject: TextView = root.findViewById(R.id.radInject)
    private val injectBox: LinearLayout = root.findViewById(R.id.radInjectBox)
    private val offBox: LinearLayout = root.findViewById(R.id.radOffBox)
    private val offIcon: ImageView = root.findViewById(R.id.radOffIcon)
    private val offState: TextView = root.findViewById(R.id.radOffState)
    private val offHint: TextView = root.findViewById(R.id.radOffHint)
    private val captureBtn: TextView = root.findViewById(R.id.radCapture)
    private val deauthBtn: TextView = root.findViewById(R.id.radDeauth)

    init {
        root.findViewById<View>(R.id.radStatus).setOnClickListener { run("status", "") }
        root.findViewById<View>(R.id.radSniff).setOnClickListener { run("sniff", "30") }
        root.findViewById<View>(R.id.radKismetOn).setOnClickListener { run("kismet-start", "") }
        root.findViewById<View>(R.id.radKismetOff).setOnClickListener {
            ConfirmSheet(act, "Arrêter Kismet ?", "Arrêter Kismet")
                .kicker("WIDS AVEUGLÉ · AUDITÉ")
                .message("Le capteur ne détectera plus rien pendant l'arrêt. L'action est tracée dans Wazuh (règle 100733).")
                .show { run("kismet-stop", "") }
        }
        root.findViewById<View>(R.id.radEspScan).setOnClickListener { run("esp-scan", "") }
        root.findViewById<View>(R.id.radEspStatus).setOnClickListener { run("esp-status", "") }
        captureBtn.setOnClickListener { captureSheet() }
        deauthBtn.setOnClickListener { deauthSheet() }
        applyLock()
    }

    /** Appelé à chaque affichage de l'écran : premier « Statut » automatique (pilote le verrou). */
    fun onShown() {
        if (!statusLoaded) { statusLoaded = true; run("status", "") }
    }

    private fun run(action: String, args: String) {
        append("→ $action${if (args.isNotEmpty()) " $args" else ""}…")
        ui.launch {
            val r = RadioClient.call(ctx, action, args)
            if (action == "status") applyStatus(r)
            val head = if (r.ok) "✓" else "✗"
            val detail = r.error ?: r.raw.replace("\n", " · ")
            append("$head $action : $detail")
        }
    }

    private fun applyStatus(r: RadioClient.Result) {
        last.text = "dernier statut ${hhmm.format(Date())}"
        if (!r.ok) {
            setChip("INJOIGNABLE", R.drawable.chip_danger, R.color.danger_soft)
            return
        }
        val f = r.fields
        val inj = f["inject"] ?: "none"
        injectAvailable = inj != "none"
        val k = f["kismet"] ?: f["kismet_state"]
        kismet.text = k ?: "—"
        uplink.text = f["uplink"] ?: f["wlan0"] ?: "—"
        inject.text = if (injectAvailable) inj else "aucune"
        inject.setTextColor(color(if (injectAvailable) R.color.ok_soft else R.color.warn_soft))
        injectBox.setBackgroundResource(if (injectAvailable) R.drawable.tile else R.drawable.card_warn)
        val kOn = k != null && k.lowercase(Locale.ROOT).let { it.contains("up") || it.contains("on") || it.contains("active") || it.contains("run") }
        if (k == null) setChip("EN LIGNE", R.drawable.chip_ok, R.color.ok_soft)
        else if (kOn) setChip("KISMET ON", R.drawable.chip_ok, R.color.ok_soft)
        else setChip("KISMET OFF", R.drawable.chip_warn, R.color.warn_soft)
        applyLock()
    }

    private fun setChip(text: String, bg: Int, fg: Int) {
        chip.text = text; chip.setBackgroundResource(bg); chip.setTextColor(color(fg))
    }

    private fun applyLock() {
        for (b in listOf(captureBtn, deauthBtn)) b.isEnabled = injectAvailable
        if (injectAvailable) {
            offBox.setBackgroundResource(R.drawable.card)
            offState.text = "PRÊT"; offState.setTextColor(color(R.color.warn_soft))
            offIcon.setImageResource(R.drawable.ic_alert_tri)
            offHint.text = "Carte d'injection présente. Liste blanche de BSSID sur le Pi, chaque trame auditée dans Wazuh."
        } else {
            offBox.setBackgroundResource(R.drawable.card_locked)
            offState.text = "VERROUILLÉ"; offState.setTextColor(color(R.color.danger_soft))
            offIcon.setImageResource(R.drawable.ic_lock)
            offHint.text = "Branche la carte AR9271 pour débloquer. Liste blanche de BSSID sur le Pi, chaque trame auditée dans Wazuh."
        }
    }

    private fun captureSheet() {
        if (!injectAvailable) { toast("Pas de carte d'injection — branche l'AR9271"); return }
        ConfirmSheet(act, "Capture handshake", "Lancer la capture")
            .kicker("ACTION OFFENSIVE · AUDITÉE")
            .field("bssid", "BSSID de l'AP (liste blanche)", "70:3A:D8:4C:09:E0")
            .field("ch", "Canal", "1", numeric = true)
            .show { v -> run("capture-handshake", "${v["bssid"]} ${v["ch"]}") }
    }

    private fun deauthSheet() {
        if (!injectAvailable) { toast("Pas de carte d'injection — branche l'AR9271"); return }
        ConfirmSheet(act, "Deauth-test · 5 trames", "Lancer les 5 trames")
            .kicker("ACTION OFFENSIVE · AUDITÉE")
            .field("bssid", "BSSID de l'AP (liste blanche)", "70:3A:D8:4C:09:E0")
            .field("client", "MAC du client ciblé", "aa:bb:cc:dd:ee:ff")
            .field("ch", "Canal", "1", numeric = true)
            .warn("Deauth ciblé par défaut. Un broadcast (ff:ff:ff:ff:ff:ff, à saisir explicitement) éjecte aussi le capteur et les autres clients du canal (incident du 18/11).")
            .show { v ->
                val client = v["client"].orEmpty()
                if (client.isBlank()) { toast("Indique la MAC du client (ou ff:ff:ff:ff:ff:ff pour un broadcast)"); return@show }
                run("deauth-test", "${v["bssid"]} $client ${v["ch"]}")
            }
    }

    private fun color(id: Int) = ContextCompat.getColor(act, id)
    private fun append(s: String) { log.text = (log.text.toString() + "\n" + s).takeLast(4000) }
    private fun toast(s: String) = Toast.makeText(act, s, Toast.LENGTH_SHORT).show()
}
