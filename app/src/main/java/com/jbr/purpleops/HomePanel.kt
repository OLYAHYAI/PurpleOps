package com.jbr.purpleops

import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Accueil (v35) : vue d'ensemble du SOC + allumage des nœuds.
 *
 * Source principale : `/metrics.json` de soc-display. Si soc-display est
 * injoignable (nœud GPU éteint) : « vue réduite » = Wazuh en direct (agents,
 * alertes) + sonde directe des nœuds. Le bloc Énergie est TOUJOURS affiché
 * (liste fixe des 3 nœuds) pour pouvoir allumer même quand tout est éteint.
 */
class HomePanel(
    private val act: AppCompatActivity,
    root: View,
    private val ui: CoroutineScope,
    private val openTab: (Int) -> Unit,
) {
    private val content: LinearLayout = root.findViewById(R.id.homeContent)
    private val source: TextView = root.findViewById(R.id.homeSource)

    // Couleurs d'état alignées sur soc-display (D4)
    private val cOk = Color.parseColor("#2ECC71")
    private val cWarn = Color.parseColor("#F5A524")
    private val cCrit = Color.parseColor("#FF4D5E")
    private val cInfo = Color.parseColor("#4DA3FF")
    private val cText = ContextCompat.getColor(act, R.color.text)
    private val cMuted = ContextCompat.getColor(act, R.color.muted)
    private val cNeon = ContextCompat.getColor(act, R.color.neon)
    private val cTrack = Color.parseColor("#2A2238")

    private var metrics: Metrics? = null
    private var wz: WzSummary? = null
    private var probe: Map<String, Boolean> = emptyMap()
    private var metricsErr: String? = null
    private val wakeStatus = mutableMapOf<String, String>()   // cible -> texte d'état du réveil
    private var agentsOpen = false
    private var loading = false
    private val handler = Handler(Looper.getMainLooper())
    private val tick = object : Runnable {
        override fun run() { refresh(); handler.postDelayed(this, 30_000) }
    }

    private fun dp(v: Int) = (v * act.resources.displayMetrics.density).toInt()

    /** Appelé quand l'Accueil devient visible / invisible. */
    fun setVisible(visible: Boolean) {
        handler.removeCallbacks(tick)
        if (visible) handler.post(tick)
    }

    fun refresh() {
        if (loading) return
        loading = true
        if (metrics == null && wz == null) render()
        ui.launch {
            val m = MetricsClient.load(act.applicationContext)
            metrics = m.getOrNull()
            metricsErr = m.exceptionOrNull()?.let { it.message ?: it.javaClass.simpleName }
            if (metrics == null) {
                // Vue réduite : Wazuh direct + sonde des nœuds
                wz = (WazuhClient.load(act.applicationContext) as? WzResult.Ok)?.summary
                probe = Cluster.probe()
            } else {
                // /metrics.json ne couvre que le cluster : PBS (hors cluster) est sondé directement.
                probe = Cluster.probe(Cluster.NODES.filter { !it.inCluster })
            }
            loading = false
            render()
        }
    }

    // ------------------------------------------------------------ rendu

    private fun render() {
        content.removeAllViews()
        val m = metrics
        source.text = when {
            m != null -> "soc-display · ${age(m.collectedAt)}"
            loading -> "chargement…"
            else -> "vue réduite"
        }
        source.setTextColor(if (m != null) cMuted else cWarn)
        hero(m)
        energy(m)
        if (m != null) { soar(m); alerts(m); mitre(m); net(m) } else {
            alertsFallback()
            content.addView(note("soc-display injoignable (${metricsErr ?: "?"}) : chaîne SOAR, MITRE et réseau indisponibles. Il tourne sur le nœud GPU « proxmox »."))
        }
        agents(m)
        shortcuts()
    }

    private fun hero(m: Metrics?) {
        val c = card(R.drawable.hero)
        c.addView(label("État du SOC"))
        val (txt, col, why) = when {
            m != null -> Triple(m.levelText.ifBlank { "—" }, levelColor(m.level),
                if (m.reasons.isEmpty()) "tous les indicateurs sont au vert" else m.reasons.joinToString(" · "))
            wz != null -> Triple(wz!!.statusLabel, if (wz!!.statusLabel == "Protégé") cOk else if (wz!!.statusLabel == "Attention") cWarn else cCrit,
                "Wazuh en direct (score ${wz!!.score})")
            loading -> Triple("…", cMuted, "chargement")
            else -> Triple("HORS LIGNE", cCrit, "ni soc-display ni Wazuh ne répondent : nœuds éteints ? Bloc Énergie ci-dessous.")
        }
        c.addView(text(txt, 26f, col, bold = true))
        c.addView(text(why, 12.5f, cMuted).apply { setPadding(0, dp(2), 0, 0) })
        if (m != null) {
            val chips = row().apply { setPadding(0, dp(10), 0, 0) }
            m.quorate?.let { chips.addView(chip(if (it) "Quorum OK" else "Quorum PERDU", if (it) cOk else cCrit)) }
            chips.addView(chip("Alertes ≥10 : ${m.crit + m.high} (24 h)", if (m.crit + m.high > 0) cWarn else cOk))
            if (m.crit > 0) chips.addView(chip("≥12 : ${m.crit}", cCrit))
            c.addView(chips)
        }
        content.addView(c)
    }

    private fun energy(m: Metrics?) {
        val c = card()
        val right = m?.powerW?.let { "${it.toInt()} W" + (m.dayAvgW?.let { a -> " · moy. ${a.toInt()} W" } ?: "") } ?: ""
        c.addView(label("Nœuds & énergie", right))
        var anyNotUp = false
        for (n in Cluster.NODES) {
            val mn = m?.nodes?.firstOrNull { it.name.equals(n.pveName, true) }
            val up: Boolean? = mn?.online ?: probe[n.target]
            if (up != true && n.inCluster) anyNotUp = true
            val box = LinearLayout(act).apply {
                orientation = LinearLayout.VERTICAL
                setBackgroundResource(R.drawable.tile)
                setPadding(dp(12), dp(10), dp(12), dp(10))
                if (up != true) alpha = 0.85f
            }
            box.layoutParams = lp().apply { bottomMargin = dp(8) }
            val head = row()
            head.addView(dot(when (up) { true -> cOk; false -> cMuted; null -> cMuted }))
            head.addView(text(n.pveName, 15f, cText, bold = true).apply { setPadding(dp(8), 0, dp(8), 0) })
            head.addView(text(n.role, 11.5f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(0, -2, 1f); maxLines = 1; ellipsize = TextUtils.TruncateAt.END })
            head.addView(text(when { mn != null && mn.online -> upt(mn.uptime); up == true -> "en ligne"; up == false -> if (n.inCluster) "éteint / injoignable" else "en veille"; else -> "…" }, 11.5f, cMuted))
            box.addView(head)
            if (mn != null && mn.online) {
                box.addView(barRow("CPU", mn.cpu, "${(mn.cpu * 100).toInt()} %"))
                box.addView(barRow("RAM", if (mn.maxmem > 0) mn.mem.toDouble() / mn.maxmem else 0.0, "${gib(mn.mem)}/${gib(mn.maxmem)} Go"))
            }
            val foot = row().apply { setPadding(0, dp(8), 0, 0) }
            foot.addView(text(
                wakeStatus[n.target] ?: if (mn != null && mn.online) "${mn.guestsRunning}/${mn.guestsTotal} invités en marche" else "réveil via wol-pi (${n.via})",
                11.5f, if (wakeStatus[n.target] != null) cInfo else cMuted
            ).apply { layoutParams = LinearLayout.LayoutParams(0, -2, 1f) })
            if (up == true) foot.addView(button(if (n.inCluster) "Proxmox ↗" else "PBS ↗", primary = false) { openUrl(Cluster.proxmoxUrl(n)) })
            else foot.addView(button("Allumer", primary = true) { confirmWake(listOf(n)) })
            box.addView(foot)
            c.addView(box)
        }
        if (anyNotUp) c.addView(button("Tout allumer", primary = true) { confirmWake(Cluster.CLUSTER) }.apply {
            layoutParams = lp().apply { topMargin = dp(2) }; gravity = Gravity.CENTER
        })
        c.addView(note("Allumer : wol-pi (audit Wazuh 100721+) · Éteindre : interface Proxmox du nœud (↗)."))
        content.addView(c)
    }

    private fun soar(m: Metrics) {
        if (m.chain.isEmpty()) return
        val c = card()
        c.addView(label("Chaîne SOAR", "24 h : ${fmt(m.total24h)} → n8n ${fmt(m.n8n24h)} → IA ${m.ia24h}"))
        val r = row().apply { gravity = Gravity.CENTER_VERTICAL }
        m.chain.forEachIndexed { i, l ->
            if (i > 0) r.addView(text("→", 12f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(0, -2, 0.4f); gravity = Gravity.CENTER })
            val col = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER_HORIZONTAL; layoutParams = LinearLayout.LayoutParams(0, -2, 1f) }
            col.addView(dot(when (l.state) { 0 -> cOk; 1 -> cWarn; 2 -> cCrit; else -> cMuted }, 12))
            col.addView(text(l.name, 11.5f, cText).apply { gravity = Gravity.CENTER; setPadding(0, dp(4), 0, 0) })
            r.addView(col)
        }
        c.addView(r)
        content.addView(c)
    }

    private fun alerts(m: Metrics) {
        val c = card()
        c.addView(label("Alertes 24 h"))
        val tiles = row()
        tiles.addView(kpi(fmt(m.total24h), "reçues", cText))
        tiles.addView(kpi("${m.crit + m.high}", "niveau ≥ 10", if (m.crit + m.high > 0) cWarn else cOk))
        tiles.addView(kpi("${m.crit}", "niveau ≥ 12", if (m.crit > 0) cCrit else cOk, last = true))
        c.addView(tiles)
        for (a in m.recent.take(3)) c.addView(alertRow(a.level, hhmm(a.ts), a.desc.ifBlank { a.agent }))
        if (m.recent.isEmpty()) c.addView(note("Aucune alerte de niveau ≥ 10 récente."))
        content.addView(c)
    }

    private fun alertsFallback() {
        val s = wz ?: return
        val c = card()
        c.addView(label("Alertes 24 h", "Wazuh direct"))
        c.addView(text("${s.alerts24h} alertes sur 24 h", 14f, cText))
        for (a in s.alerts.take(3)) c.addView(alertRow(a.level, "", a.desc))
        content.addView(c)
    }

    private fun mitre(m: Metrics) {
        val c = card()
        c.addView(label("MITRE ATT&CK · 7 j", "${m.mitreCovered}/${m.mitreTotal} tactiques"))
        c.addView(bar(if (m.mitreTotal > 0) m.mitreCovered.toDouble() / m.mitreTotal else 0.0, cNeon).apply { layoutParams = lp(dp(8)).apply { topMargin = dp(2); bottomMargin = dp(8) } })
        c.addView(text(if (m.blindSpots.isEmpty()) "Aucun angle mort." else "Angles morts : " + m.blindSpots.joinToString(", "), 12f, cMuted))
        content.addView(c)
    }

    private fun net(m: Metrics) {
        if (m.net.isEmpty() && m.dnsOk == null) return
        val c = card()
        c.addView(label("Réseau"))
        val r = row()
        val gw = m.net.firstOrNull { it.name.contains("asserelle", true) }
        val inet = m.net.firstOrNull { it !== gw }
        r.addView(kpi(ms(inet?.lastMs), "Internet", cText))
        r.addView(kpi(ms(gw?.lastMs), "passerelle", cText))
        r.addView(kpi(if (m.dnsOk == true) "OK" else if (m.dnsOk == false) "KO" else "—", "DNS ${ms(m.dnsMs)}", if (m.dnsOk == true) cOk else cCrit, last = true))
        c.addView(r)
        content.addView(c)
    }

    private fun agents(m: Metrics?) {
        val c = card()
        val active: Int; val total: Int; val down: List<String>
        if (m != null) { active = m.agentsActive; total = m.agentsTotal; down = m.agentsDown }
        else if (wz != null) { active = wz!!.active; total = wz!!.total; down = wz!!.agents.filter { !it.status.equals("active", true) }.map { "${it.name} (${it.id})" } }
        else return
        val head = row().apply { gravity = Gravity.CENTER_VERTICAL }
        head.addView(text("Agents", 14f, cText, bold = true).apply { setPadding(0, 0, dp(8), 0) })
        head.addView(chip("$active actifs", cOk))
        if (total - active > 0) head.addView(chip("${total - active} déconnectés", cCrit))
        head.addView(View(act).apply { layoutParams = LinearLayout.LayoutParams(0, 1, 1f) })
        head.addView(text(if (agentsOpen) "replier ⌃" else "déplier ⌄", 12.5f, cNeon))
        head.setOnClickListener { agentsOpen = !agentsOpen; render() }
        c.addView(head)
        if (agentsOpen) {
            val list = wz?.agents
            if (list != null && m == null) list.forEach { a -> c.addView(agentRow(a.name, "${a.status} · ${a.ip}", a.status.equals("active", true))) }
            else if (down.isEmpty()) c.addView(note("Aucun agent déconnecté."))
            else down.forEach { c.addView(agentRow(it, "déconnecté", false)) }
        }
        content.addView(c)
    }

    private fun shortcuts() {
        val c = card()
        c.addView(label("Raccourcis"))
        val r = row()
        listOf(Triple("▣", "Écran USB", R.id.nav_screen), Triple("◉", "Nikon", R.id.nav_camera), Triple(">_", "Catalogue Kali", R.id.nav_kali))
            .forEachIndexed { i, (g, t, id) ->
                val b = LinearLayout(act).apply {
                    orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER
                    setBackgroundResource(R.drawable.btn_neon); setPadding(dp(6), dp(10), dp(6), dp(10))
                    layoutParams = LinearLayout.LayoutParams(0, -2, 1f).apply { if (i < 2) marginEnd = dp(8) }
                    setOnClickListener { openTab(id) }
                }
                b.addView(text(g, 17f, cNeon, bold = true).apply { gravity = Gravity.CENTER })
                b.addView(text(t, 12f, cText).apply { gravity = Gravity.CENTER })
                r.addView(b)
            }
        c.addView(r)
        content.addView(c)
    }

    // ------------------------------------------------------------ réveil

    private fun confirmWake(nodes: List<NodeDef>) {
        val ctx = act.applicationContext
        if (!WolClient.isPaired(ctx)) {
            AlertDialog.Builder(act).setTitle("wol-pi non appairé")
                .setMessage("Appaire d'abord le téléphone avec wol-pi : Réglages › Réveil des nœuds.")
                .setPositiveButton("Réglages") { _, _ -> openTab(R.id.nav_settings) }
                .setNegativeButton("Annuler", null).show()
            return
        }
        val names = nodes.joinToString(", ") { it.pveName }
        AlertDialog.Builder(act)
            .setTitle(if (nodes.size > 1) "Tout allumer ?" else "Allumer ${nodes[0].pveName} ?")
            .setMessage("Paquet magique envoyé par wol-pi à : $names.\nUn nœud déjà allumé répond « déjà allumé » (aucun effet). Chaque demande est auditée dans Wazuh.")
            .setPositiveButton("Allumer") { _, _ -> doWake(nodes) }
            .setNegativeButton("Annuler", null).show()
    }

    private fun doWake(nodes: List<NodeDef>) {
        val ctx = act.applicationContext
        ui.launch {
            val sent = mutableListOf<NodeDef>()
            for (n in nodes) {
                wakeStatus[n.target] = "demande envoyée à wol-pi…"; render()
                val r = WolClient.wake(ctx, n.target)
                WolClient.audit(n.target, r)
                wakeStatus[n.target] = when {
                    r == "sent" -> "paquet magique envoyé, démarrage…"
                    r == "already_up" -> "déjà allumé"
                    r == "no_link" -> "pas de lien réseau vers le nœud (${n.via})"
                    r == "rejected" -> "refusé par wol-pi"
                    r == "error" -> "échec d'envoi (etherwake)"
                    else -> r.removePrefix("ssh_error: ")
                }
                if (r == "sent") sent += n
                render()
            }
            // Suivi : l'interface web répond-elle (8006 PVE / 8007 PBS) ? 18 × 10 s = 3 min
            for (i in 1..18) {
                if (sent.isEmpty()) break
                delay(10_000)
                val p = Cluster.probe()
                val iter = sent.iterator()
                while (iter.hasNext()) {
                    val n = iter.next()
                    if (p[n.target] == true) { wakeStatus[n.target] = "allumé ✓ (interface :${n.port} joignable)"; iter.remove() }
                    else wakeStatus[n.target] = "démarrage… ${i * 10} s"
                }
                render()
            }
            for (n in sent) wakeStatus[n.target] = "pas de réponse après 3 min : vérifier alimentation / BIOS"
            render()
            handler.postDelayed({ wakeStatus.clear(); refresh() }, 60_000)
        }
    }

    // ------------------------------------------------------------ petits composants

    private fun lp(h: Int = ViewGroup.LayoutParams.WRAP_CONTENT) = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, h)

    private fun card(bg: Int = R.drawable.card) = LinearLayout(act).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundResource(bg)
        setPadding(dp(14), dp(12), dp(14), dp(12))
        layoutParams = lp().apply { bottomMargin = dp(10) }
    }

    private fun row() = LinearLayout(act).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL; layoutParams = lp() }

    private fun text(s: String, size: Float, color: Int, bold: Boolean = false) = TextView(act).apply {
        text = s; textSize = size; setTextColor(color); if (bold) setTypeface(typeface, Typeface.BOLD)
    }

    private fun label(s: String, right: String = "") = row().apply {
        setPadding(0, 0, 0, dp(8))
        addView(text(s.uppercase(), 10.5f, cNeon, bold = true).apply { letterSpacing = 0.12f; layoutParams = LinearLayout.LayoutParams(0, -2, 1f) })
        if (right.isNotEmpty()) addView(text(right, 11.5f, cMuted))
    }

    private fun note(s: String) = text(s, 11f, Color.parseColor("#6F6884")).apply { setPadding(0, dp(6), 0, 0) }

    private fun chip(s: String, color: Int) = text(s, 11f, color).apply {
        background = GradientDrawable().apply { cornerRadius = dp(10).toFloat(); setColor(Color.parseColor("#1E1830")); setStroke(dp(1), adjustAlpha(color, 0.45f)) }
        setPadding(dp(8), dp(3), dp(8), dp(3))
        layoutParams = LinearLayout.LayoutParams(-2, -2).apply { marginEnd = dp(6) }
    }

    private fun dot(color: Int, sizeDp: Int = 9) = View(act).apply {
        background = GradientDrawable().apply { shape = GradientDrawable.OVAL; setColor(color) }
        layoutParams = LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp)).apply { gravity = Gravity.CENTER }
    }

    private fun bar(frac: Double, color: Int) = LinearLayout(act).apply {
        background = GradientDrawable().apply { cornerRadius = dp(3).toFloat(); setColor(cTrack) }
        val f = frac.coerceIn(0.0, 1.0).toFloat()
        addView(View(act).apply {
            background = GradientDrawable().apply { cornerRadius = dp(3).toFloat(); setColor(color) }
            layoutParams = LinearLayout.LayoutParams(0, -1, f.coerceAtLeast(0.001f))
        })
        addView(View(act).apply { layoutParams = LinearLayout.LayoutParams(0, -1, (1f - f).coerceAtLeast(0.001f)) })
    }

    private fun barRow(name: String, frac: Double, value: String) = row().apply {
        setPadding(0, dp(6), 0, 0)
        addView(text(name, 11.5f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(dp(36), -2) })
        addView(bar(frac, cNeon).apply { layoutParams = LinearLayout.LayoutParams(0, dp(6), 1f) })
        addView(text(value, 11.5f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(dp(78), -2); gravity = Gravity.END })
    }

    private fun button(s: String, primary: Boolean, onClick: () -> Unit) = TextView(act).apply {
        text = s; textSize = 12.5f; setTypeface(typeface, Typeface.BOLD)
        setTextColor(if (primary) Color.WHITE else cNeon)
        setBackgroundResource(if (primary) R.drawable.btn_accent else R.drawable.btn_neon)
        setPadding(dp(14), dp(8), dp(14), dp(8))
        minHeight = dp(40); gravity = Gravity.CENTER
        setOnClickListener { onClick() }
    }

    private fun kpi(v: String, l: String, color: Int, last: Boolean = false) = LinearLayout(act).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundResource(R.drawable.tile)
        setPadding(dp(10), dp(8), dp(10), dp(8))
        layoutParams = LinearLayout.LayoutParams(0, -2, 1f).apply { if (!last) marginEnd = dp(8) }
        addView(text(v, 19f, color, bold = true))
        addView(text(l, 11f, cMuted))
    }

    private fun alertRow(level: Int, t: String, desc: String) = row().apply {
        setPadding(0, dp(7), 0, 0)
        addView(text("$level", 12f, Color.parseColor("#120E1A"), bold = true).apply {
            gravity = Gravity.CENTER
            background = GradientDrawable().apply { cornerRadius = dp(6).toFloat(); setColor(levelColorForRule(level)) }
            layoutParams = LinearLayout.LayoutParams(dp(28), -2).apply { marginEnd = dp(8) }
        })
        if (t.isNotEmpty()) addView(text(t, 11.5f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(dp(40), -2) })
        addView(text(desc, 12.5f, cText).apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END; layoutParams = LinearLayout.LayoutParams(0, -2, 1f) })
    }

    private fun agentRow(name: String, sub: String, ok: Boolean) = row().apply {
        setPadding(0, dp(8), 0, 0)
        addView(dot(if (ok) cOk else cCrit))
        addView(text(name, 13f, cText).apply { setPadding(dp(8), 0, dp(8), 0); layoutParams = LinearLayout.LayoutParams(0, -2, 1f) })
        addView(text(sub, 11.5f, cMuted))
    }

    private fun openUrl(u: String) = try {
        act.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(u)))
    } catch (e: Exception) { }

    private fun levelColor(l: Int) = when (l) { 0 -> cOk; 1 -> cWarn; else -> cCrit }
    private fun levelColorForRule(l: Int) = when { l >= 12 -> cCrit; l >= 10 -> cWarn; l >= 7 -> Color.parseColor("#E8D44D"); else -> cMuted }
    private fun adjustAlpha(c: Int, a: Float) = Color.argb((255 * a).toInt(), Color.red(c), Color.green(c), Color.blue(c))
    private fun gib(b: Long) = String.format(java.util.Locale.FRANCE, "%.0f", b / 1073741824.0)
    private fun ms(v: Double?) = v?.let { if (it < 10) String.format(java.util.Locale.FRANCE, "%.1f ms", it) else "${it.toInt()} ms" } ?: "—"
    private fun fmt(n: Long) = String.format(java.util.Locale.FRANCE, "%,d", n).replace(' ', ' ').replace(' ', ' ')
    private fun upt(s: Long): String { val d = s / 86400; val h = (s % 86400) / 3600; return if (d > 0) "$d j $h h" else "$h h ${(s % 3600) / 60} min" }

    private fun parseTs(ts: String?): java.time.OffsetDateTime? = try { ts?.let { java.time.OffsetDateTime.parse(it) } } catch (e: Exception) { null }
    private fun hhmm(ts: String?): String = parseTs(ts)?.atZoneSameInstant(java.time.ZoneId.systemDefault())
        ?.let { String.format("%02d:%02d", it.hour, it.minute) } ?: ""
    private fun age(ts: String?): String {
        val t = parseTs(ts) ?: return "?"
        val s = java.time.Duration.between(t.toInstant(), java.time.Instant.now()).seconds
        return when { s < 60 -> "il y a $s s"; s < 3600 -> "il y a ${s / 60} min"; else -> "il y a ${s / 3600} h" }
    }
}
