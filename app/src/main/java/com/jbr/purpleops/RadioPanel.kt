package com.jbr.purpleops

import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Écran radio (wol-pi) : connect / sniff / attaque depuis un bouton.
 *
 * Appelle le webhook n8n ([RadioClient]). Les actions offensives
 * (capture-handshake, deauth-test) restent grisées tant que le dernier
 * « Statut » rapporte `inject=none` (pas de carte AR9271) ; de toute façon le
 * Pi applique l'allowlist de BSSID et audite dans Wazuh. Périmètre : TON réseau.
 */
class RadioPanel(private val act: AppCompatActivity, private val ui: CoroutineScope) {

    private val ctx: Context get() = act.applicationContext
    private var injectAvailable = false
    private lateinit var log: TextView
    private lateinit var captureBtn: TextView
    private lateinit var deauthBtn: TextView

    private fun dp(v: Int) = (v * act.resources.displayMetrics.density).toInt()

    fun open() {
        val root = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(18), dp(8), dp(18), 0) }

        val urlInput = EditText(act).apply {
            setText(RadioClient.url(ctx)); setSingleLine(); textSize = 12f
            setTextColor(Color.parseColor("#EDE9F5")); setHintTextColor(Color.parseColor("#8A7FA5"))
            inputType = InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_URI
        }
        root.addView(label("Webhook n8n")); root.addView(urlInput)

        val grid = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; setPadding(0, dp(10), 0, 0) }
        grid.addView(btn("Statut") { run("status", "") })
        grid.addView(btn("Sniff 30 s") { run("sniff", "30") })
        grid.addView(btn("Kismet ON") { run("kismet-start", "") })
        grid.addView(btn("Kismet OFF") { confirm("Arrêter Kismet ?", "Le WIDS ne détectera plus pendant l'arrêt. Action auditée.") { run("kismet-stop", "") } })
        captureBtn = btn("Capture handshake…") { captureDialog() }
        deauthBtn = btn("Deauth-test (5 trames)…") { deauthDialog() }
        grid.addView(captureBtn); grid.addView(deauthBtn)
        root.addView(grid)

        log = TextView(act).apply {
            text = "Touche « Statut » pour l'état du capteur."
            setTextColor(Color.parseColor("#8A7FA5")); textSize = 12f; typeface = Typeface.MONOSPACE
            setPadding(0, dp(10), 0, dp(4))
        }
        root.addView(log)

        applyGrey()

        val scroller = ScrollView(act).apply { addView(root) }
        AlertDialog.Builder(act).setTitle("Radio — wol-pi").setView(scroller)
            .setPositiveButton("Fermer", null)
            .setNeutralButton("Enregistrer l'URL") { _, _ -> RadioClient.setUrl(ctx, urlInput.text.toString()); toast("URL enregistrée") }
            .show()

        run("status", "") // état à l'ouverture (pilote le grisage)
    }

    private fun run(action: String, args: String) {
        append("→ $action${if (args.isNotEmpty()) " $args" else ""}…")
        ui.launch {
            val r = RadioClient.call(ctx, action, args)
            if (action == "status") { injectAvailable = (r.fields["inject"] ?: "none") != "none"; applyGrey() }
            val head = if (r.ok) "✓" else "✗"
            val detail = r.error ?: r.raw.replace("\n", " · ")
            append("$head $action : $detail")
        }
    }

    private fun captureDialog() {
        if (!injectAvailable) { toast("Pas de carte d'injection — branche l'AR9271"); return }
        val bssid = field("BSSID cible (ex 70:3A:D8:4C:09:E0)")
        val ch = field("Canal (ex 1)").apply { inputType = InputType.TYPE_CLASS_NUMBER }
        val box = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(18), dp(8), dp(18), 0) }
        box.addView(label("BSSID AP")); box.addView(bssid); box.addView(label("Canal")); box.addView(ch)
        AlertDialog.Builder(act).setTitle("Capture handshake").setView(box)
            .setPositiveButton("Lancer") { _, _ ->
                run("capture-handshake", "${bssid.text.toString().trim()} ${ch.text.toString().trim()}")
            }
            .setNegativeButton("Annuler", null).show()
    }

    private fun deauthDialog() {
        if (!injectAvailable) { toast("Pas de carte d'injection — branche l'AR9271"); return }
        val bssid = field("BSSID AP (ex 70:3A:D8:4C:09:E0)")
        val client = field("Client MAC (vide = broadcast)")
        val ch = field("Canal (ex 1)").apply { inputType = InputType.TYPE_CLASS_NUMBER }
        val box = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(18), dp(8), dp(18), 0) }
        box.addView(label("BSSID AP")); box.addView(bssid); box.addView(label("Client")); box.addView(client); box.addView(label("Canal")); box.addView(ch)
        AlertDialog.Builder(act).setTitle("Deauth-test (TON réseau)").setView(box)
            .setPositiveButton("Lancer") { _, _ ->
                val c = client.text.toString().trim().ifBlank { "ff:ff:ff:ff:ff:ff" }
                run("deauth-test", "${bssid.text.toString().trim()} $c ${ch.text.toString().trim()}")
            }
            .setNegativeButton("Annuler", null).show()
    }

    private fun applyGrey() {
        for (b in listOf(captureBtn, deauthBtn)) {
            b.isEnabled = injectAvailable
            b.alpha = if (injectAvailable) 1f else 0.4f
        }
    }

    // --- helpers UI ---
    private fun label(t: String) = TextView(act).apply {
        text = t; setTextColor(Color.parseColor("#8A7FA5")); textSize = 12f; setPadding(0, dp(6), 0, dp(2))
    }

    private fun field(hint: String) = EditText(act).apply {
        this.hint = hint; setSingleLine(); setTextColor(Color.parseColor("#EDE9F5")); setHintTextColor(Color.parseColor("#8A7FA5"))
        inputType = InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
    }

    private fun btn(text: String, onClick: () -> Unit): TextView = TextView(act).apply {
        this.text = text; setTextColor(Color.parseColor("#EDE9F5")); textSize = 15f
        setBackgroundResource(R.drawable.btn_neon); gravity = Gravity.CENTER
        setPadding(dp(14), dp(12), dp(14), dp(12)); isClickable = true
        layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply { bottomMargin = dp(8) }
        setOnClickListener { onClick() }
    }

    private fun confirm(title: String, msg: String, onYes: () -> Unit) {
        AlertDialog.Builder(act).setTitle(title).setMessage(msg)
            .setPositiveButton("Oui") { _, _ -> onYes() }.setNegativeButton("Non", null).show()
    }

    private fun append(s: String) { log.text = (log.text.toString() + "\n" + s).takeLast(4000) }
    private fun toast(s: String) = Toast.makeText(act, s, Toast.LENGTH_SHORT).show()
}
