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
    /** Ouvre le chat #soc avec un message prérempli (appui long sur une alerte). */
    private val askSoc: (String) -> Unit = {},
) {
    private val content: LinearLayout = root.findViewById(R.id.homeContent)
    private val source: TextView = root.findViewById(R.id.homeSource)

    init {
        root.findViewById<TextView>(R.id.homeKicker).text = "SOC · MAISON · v${BuildConfig.VERSION_NAME}"
    }

    private val n = Neon(act)

    // Couleurs d'état alignées sur soc-display (D4)
    private val cOk = Color.parseColor("#2ECC71")
    private val cWarn = Color.parseColor("#F5A524")
    private val cCrit = Color.parseColor("#FF4D5E")
    private val cInfo = Color.parseColor("#4DA3FF")
    private val cText = ContextCompat.getColor(act, R.color.text)
    private val cMuted = ContextCompat.getColor(act, R.color.muted)
    private val cNeon = ContextCompat.getColor(act, R.color.neon)
    private val cTrack = Color.parseColor("#1E2731")

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

    // ------------------------------------------------------------ rendu (« Neon Ops »)

    private fun render() {
        content.removeAllViews()
        val m = metrics
        source.text = when {
            m != null -> "SOC-DISPLAY · ${age(m.collectedAt).uppercase()}"
            loading -> "CHARGEMENT…"
            else -> "VUE RÉDUITE"
        }
        source.setBackgroundResource(if (m != null) R.drawable.chip_ok else R.drawable.chip_warn)
        source.setTextColor(ContextCompat.getColor(act, if (m != null) R.color.ok_soft else R.color.warn_soft))
        hero(m)
        energy(m)
        if (m != null) { soar(m); alerts(m); mitre(m); net(m) } else {
            alertsFallback()
            content.addView(note("soc-display injoignable (${metricsErr ?: "?"}) : chaîne SOAR, MITRE et réseau indisponibles. Il tourne sur le nœud GPU « proxmox »."))
        }
        agents(m)
        shortcuts()
    }

    /** Carte « Posture SOC » : jauge circulaire + indicateurs clés. */
    private fun hero(m: Metrics?) {
        val c = n.card(R.drawable.hero, 16, 16)
        val r = n.row()
        val (txt, col, why, frac) = when {
            m != null -> Quad(m.levelText.ifBlank { "—" }, levelColor(m.level),
                if (m.reasons.isEmpty()) "tous les indicateurs sont au vert" else m.reasons.joinToString(" · "),
                when (m.level) { 0 -> 0.25f; 1 -> 0.6f; else -> 0.95f })
            wz != null -> Quad(wz!!.statusLabel, if (wz!!.statusLabel == "Protégé") cOk else if (wz!!.statusLabel == "Attention") cWarn else cCrit,
                "Wazuh en direct · score ${wz!!.score}", (wz!!.score / 100f))
            loading -> Quad("…", cMuted, "chargement", 0f)
            else -> Quad("HORS LIGNE", cCrit, "ni soc-display ni Wazuh ne répondent : nœuds éteints ? Bloc Énergie ci-dessous.", 1f)
        }
        // Jauge
        val g = android.widget.FrameLayout(act).apply { layoutParams = LinearLayout.LayoutParams(n.dp(112), n.dp(112)).apply { marginEnd = n.dp(14) } }
        g.addView(GaugeView(act).apply { fraction = frac; color = col; layoutParams = android.widget.FrameLayout.LayoutParams(-1, -1) })
        val center = LinearLayout(act).apply {
            orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER
            layoutParams = android.widget.FrameLayout.LayoutParams(-1, -1)
        }
        center.addView(n.title(if (m != null) "${m.crit + m.high}" else if (wz != null) "${wz!!.score}" else "—", 28f, n.text).apply { gravity = Gravity.CENTER })
        center.addView(n.monoText(if (m != null) "≥10 · 24 H" else if (wz != null) "SCORE" else "", 9.5f, n.accentSoft).apply { gravity = Gravity.CENTER; letterSpacing = 0.1f })
        g.addView(center)
        r.addView(g)
        // Indicateurs
        val col2 = n.column().apply { layoutParams = n.weight() }
        col2.addView(n.kicker("Posture SOC"))
        col2.addView(n.title(txt, 22f, n.lighten(col)).apply { setPadding(0, n.dp(2), 0, n.dp(6)) })
        if (m != null) {
            col2.addView(kv("Alertes ≥10 · 24 h", "${m.crit + m.high}", if (m.crit + m.high > 0) cWarn else cOk))
            col2.addView(kv("Niveau ≥12 · 24 h", "${m.crit}", if (m.crit > 0) cCrit else cOk))
            m.quorate?.let { col2.addView(kv("Quorum cluster", if (it) "OK" else "PERDU", if (it) cOk else cCrit)) }
        } else if (wz != null) {
            col2.addView(kv("Alertes · 24 h", "${wz!!.alerts24h}", cText))
            col2.addView(kv("Agents actifs", "${wz!!.active}/${wz!!.total}", if (wz!!.active == wz!!.total) cOk else cWarn))
        }
        r.addView(col2)
        c.addView(r)
        c.addView(n.text(why, 12f, n.soft).apply { setPadding(0, n.dp(10), 0, 0) })
        content.addView(c)
    }

    private data class Quad(val a: String, val b: Int, val c: String, val d: Float)

    private fun kv(k: String, v: String, color: Int) = n.row().apply {
        setPadding(0, n.dp(3), 0, n.dp(3))
        addView(n.text(k, 12.5f, n.soft).apply { layoutParams = n.weight() })
        addView(n.monoText(v, 14f, n.lighten(color), bold = true))
    }

    private fun energy(m: Metrics?) {
        val right = m?.powerW?.let { "${it.toInt()} W" + (m.dayAvgW?.let { a -> " · moy. ${a.toInt()} W" } ?: "") } ?: "wol-pi"
        content.addView(n.section("Nœuds & énergie", right))
        var anyNotUp = false
        for (nd in Cluster.NODES) {
            val mn = if (nd.inCluster) m?.nodes?.firstOrNull { it.name.equals(nd.pveName, true) } else m?.pbs
            val up: Boolean? = mn?.online ?: probe[nd.target]
            if (up != true && nd.inCluster) anyNotUp = true
            val box = n.card(if (up == true) R.drawable.card else R.drawable.card_off, 14, 12)
            val head = n.row()
            head.addView(if (up == true) n.dot(cOk, 8) else n.dot(n.faint, 8, hollow = true))
            head.addView(n.monoText(nd.pveName, 14f, if (up == true) cText else n.soft, bold = true).apply { setPadding(n.dp(8), 0, n.dp(8), 0) })
            head.addView(n.ellipsize(n.text(nd.role, 11.5f, cMuted)).apply { layoutParams = n.weight() })
            if (up == true) head.addView(n.button(if (nd.inCluster) "Proxmox ↗" else "PBS ↗", Neon.Kind.SECONDARY) { openUrl(Cluster.proxmoxUrl(nd)) }.apply { minHeight = n.dp(34); textSize = 12f; setPadding(n.dp(12), n.dp(4), n.dp(12), n.dp(4)) })
            else head.addView(n.button("⏻ Allumer", Neon.Kind.SECONDARY) { confirmWake(listOf(nd)) }.apply { minHeight = n.dp(34); textSize = 12f; setPadding(n.dp(12), n.dp(4), n.dp(12), n.dp(4)) })
            box.addView(head)
            if (mn != null && mn.online) {
                box.addView(barRow("CPU", mn.cpu, "${(mn.cpu * 100).toInt()} %", if (mn.cpu > 0.8) cWarn else cNeon))
                box.addView(barRow("RAM", if (mn.maxmem > 0) mn.mem.toDouble() / mn.maxmem else 0.0, "${gib(mn.mem)}/${gib(mn.maxmem)} Go", cInfo))
            }
            val status = wakeStatus[nd.target] ?: when {
                mn != null && mn.online -> "${upt(mn.uptime)} · ${mn.guestsRunning}/${mn.guestsTotal} invités"
                !nd.inCluster && up == true -> "Proxmox Backup Server · :${nd.port}"
                up == true -> "en ligne"
                up == false -> (if (nd.inCluster) "éteint" else "en veille") + " · réveil via wol-pi (${nd.via})"
                else -> "…"
            }
            box.addView(n.monoText(status, 11f, if (wakeStatus[nd.target] != null) cInfo else cMuted).apply { setPadding(0, n.dp(8), 0, 0) })
            content.addView(box)
        }
        if (anyNotUp) content.addView(n.button("⏻ Tout allumer", Neon.Kind.PRIMARY) { confirmWake(Cluster.CLUSTER) }.apply {
            layoutParams = n.lp().apply { topMargin = n.dp(2); bottomMargin = n.dp(4) }; minHeight = n.dp(46)
        })
        content.addView(note("Allumer : wol-pi, sinon repli n8n / API Proxmox (audit Wazuh) · Éteindre : interface Proxmox du nœud (↗)."))
    }

    private fun soar(m: Metrics) {
        if (m.chain.isEmpty()) return
        content.addView(n.section("Chaîne SOAR · 24 h", m.chain.joinToString(" · ") { it.name }.take(40),
            if (m.chain.all { it.state == 0 }) cOk else cWarn))
        val c = n.card()
        val max = maxOf(1L, m.total24h).toDouble()
        c.addView(funnel("Événements", m.total24h, 1.0, cInfo))
        c.addView(funnel("→ n8n", m.n8n24h, m.n8n24h / max, cNeon))
        c.addView(funnel("→ IA", m.ia24h, m.ia24h / max, cWarn))
        // Maillons : un point d'état par composant
        val r = n.row().apply { setPadding(0, n.dp(10), 0, 0) }
        m.chain.forEachIndexed { i, l ->
            if (i > 0) r.addView(View(act).apply {
                setBackgroundColor(n.border); layoutParams = LinearLayout.LayoutParams(0, n.dp(1), 0.5f)
            })
            val colv = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; gravity = Gravity.CENTER_HORIZONTAL }
            colv.addView(n.dot(when (l.state) { 0 -> cOk; 1 -> cWarn; 2 -> cCrit; else -> cMuted }, 9))
            colv.addView(n.monoText(l.name, 10.5f, n.soft).apply { gravity = Gravity.CENTER; setPadding(0, n.dp(4), 0, 0) })
            r.addView(colv)
        }
        c.addView(r)
        content.addView(c)
    }

    private fun funnel(name: String, v: Long, frac: Double, color: Int) = n.row().apply {
        setPadding(0, n.dp(4), 0, n.dp(4))
        addView(n.text(name, 12f, n.soft).apply { layoutParams = LinearLayout.LayoutParams(n.dp(86), -2) })
        addView(n.bar(frac.coerceAtLeast(0.02), color, 10).apply { layoutParams = LinearLayout.LayoutParams(0, n.dp(10), 1f) })
        addView(n.monoText(fmt(v), 12f, cText).apply { layoutParams = LinearLayout.LayoutParams(n.dp(64), -2); gravity = Gravity.END })
    }

    private fun alerts(m: Metrics) {
        content.addView(n.section("Alertes · 24 h", "${fmt(m.total24h)} reçues"))
        val tiles = n.row().apply { layoutParams = n.lp().apply { bottomMargin = n.dp(8) } }
        tiles.addView(kpi(fmt(m.total24h), "reçues", cText))
        tiles.addView(kpi("${m.crit + m.high}", "niveau ≥ 10", if (m.crit + m.high > 0) cWarn else cOk))
        tiles.addView(kpi("${m.crit}", "niveau ≥ 12", if (m.crit > 0) cCrit else cOk, last = true))
        content.addView(tiles)
        for (a in m.recent.take(3)) content.addView(alertRow(a.level, hhmm(a.ts), a.desc.ifBlank { a.agent },
            listOfNotNull(a.agent.takeIf { it.isNotBlank() }, a.ruleId.takeIf { it.isNotBlank() }?.let { "règle $it" }, a.mitre.firstOrNull()).joinToString(" · ")).apply {
            setOnLongClickListener { askSoc(askText(a.level, a.desc, a.agent, hhmm(a.ts), a.id, a.ruleId, a.srcip, a.mitre)); true }
        })
        content.addView(note(if (m.recent.isEmpty()) "Aucune alerte de niveau ≥ 10 récente." else "Appui long sur une alerte : la soumettre à un agent dans #soc."))
    }

    private fun alertsFallback() {
        val s = wz ?: return
        content.addView(n.section("Alertes · 24 h", "Wazuh direct · ${s.alerts24h}", cWarn))
        for (a in s.alerts.take(3)) content.addView(alertRow(a.level, hhmm(a.ts), a.desc, a.agent).apply {
            setOnLongClickListener { askSoc(askText(a.level, a.desc, a.agent, hhmm(a.ts))); true }
        })
    }

    /** Bande MITRE : une case par tactique, les angles morts en pointillés rouges. */
    private fun mitre(m: Metrics) {
        val pct = if (m.mitreTotal > 0) m.mitreCovered * 100 / m.mitreTotal else 0
        content.addView(n.section("Couverture MITRE ATT&CK", "${m.mitreCovered}/${m.mitreTotal} · $pct %", n.accentSoft))
        val c = n.card()
        val total = m.mitreTotal.coerceIn(1, 20)
        val strip = n.row()
        for (i in 0 until total) {
            val covered = i < m.mitreCovered
            strip.addView(View(act).apply {
                background = android.graphics.drawable.GradientDrawable().apply {
                    cornerRadius = n.dpf(4f)
                    if (covered) setColor(n.fade(cNeon, 0.45f + 0.5f * ((i * 37) % 10) / 10f))
                    else { setColor(android.graphics.Color.TRANSPARENT); setStroke(n.dp(1), cCrit, n.dpf(3f), n.dpf(2f)) }
                }
                layoutParams = LinearLayout.LayoutParams(0, n.dp(26), 1f).apply { if (i < total - 1) marginEnd = n.dp(3) }
            })
        }
        c.addView(strip)
        c.addView(n.text(if (m.blindSpots.isEmpty()) "Aucun angle mort sur 7 jours." else "Angles morts (pointillés) : " + m.blindSpots.joinToString(", "),
            12f, if (m.blindSpots.isEmpty()) cMuted else n.c(R.color.danger_soft)).apply { setPadding(0, n.dp(10), 0, 0) })
        content.addView(c)
    }

    private fun net(m: Metrics) {
        if (m.net.isEmpty() && m.dnsOk == null) return
        content.addView(n.section("Réseau"))
        val r = n.row().apply { layoutParams = n.lp().apply { bottomMargin = n.dp(8) } }
        val gw = m.net.firstOrNull { it.name.contains("asserelle", true) }
        val inet = m.net.firstOrNull { it !== gw }
        r.addView(kpi(ms(inet?.lastMs), "Internet", cText))
        r.addView(kpi(ms(gw?.lastMs), "passerelle", cText))
        r.addView(kpi(if (m.dnsOk == true) "OK" else if (m.dnsOk == false) "KO" else "—", "DNS ${ms(m.dnsMs)}", if (m.dnsOk == true) cOk else cCrit, last = true))
        content.addView(r)
    }

    private fun agents(m: Metrics?) {
        val active: Int; val total: Int; val down: List<String>
        if (m != null) { active = m.agentsActive; total = m.agentsTotal; down = m.agentsDown }
        else if (wz != null) { active = wz!!.active; total = wz!!.total; down = wz!!.agents.filter { !it.status.equals("active", true) }.map { "${it.name} (${it.id})" } }
        else return
        content.addView(n.section("Agents Wazuh", if (agentsOpen) "replier ⌃" else "déplier ⌄", n.accentSoft).apply {
            isClickable = true; setOnClickListener { agentsOpen = !agentsOpen; render() }
        })
        val c = n.card()
        val head = n.row()
        head.addView(n.title("$active", 24f, cText))
        head.addView(n.monoText(" / $total actifs", 12f, cMuted).apply { layoutParams = n.weight() })
        if (total - active > 0) head.addView(n.chip("${total - active} déconnectés", cCrit, withDot = true))
        else head.addView(n.chip("tous actifs", cOk, withDot = true))
        head.setOnClickListener { agentsOpen = !agentsOpen; render() }
        c.addView(head)
        if (agentsOpen) {
            c.addView(n.divider())
            val list = wz?.agents
            if (list != null && m == null) list.forEach { a -> c.addView(agentRow(a.name, "${a.status} · ${a.ip}", a.status.equals("active", true))) }
            else if (down.isEmpty()) c.addView(note("Aucun agent déconnecté."))
            else down.forEach { c.addView(agentRow(it, "déconnecté", false)) }
        }
        content.addView(c)
    }

    private fun shortcuts() {
        content.addView(n.section("Raccourcis"))
        val r = n.row()
        // Nikon a quitté la barre du bas (remplacé par Chat) : il prend la première place ici.
        val items = listOf(Triple(R.drawable.ic_camera, "Nikon", R.id.nav_camera), Triple(R.drawable.ic_screen, "Écran", R.id.nav_screen),
            Triple(R.drawable.ic_kali, "Kali", R.id.nav_kali))
        items.forEachIndexed { i, (icon, t, id) ->
            r.addView(n.shortcut(icon, t) { openTab(id) }.apply {
                layoutParams = LinearLayout.LayoutParams(0, -2, 1f).apply { if (i < items.size - 1) marginEnd = n.dp(8) }
            })
        }
        content.addView(r)
    }

    // ------------------------------------------------------------ réveil

    private fun confirmWake(nodes: List<NodeDef>) {
        val ctx = act.applicationContext
        if (!WolClient.isPaired(ctx) && !WakeRelay.isConfigured(ctx)) {
            ConfirmSheet(act, "Réveil non configuré", "Ouvrir les Réglages")
                .kicker("RÉVEIL DES NŒUDS").tone(ConfirmSheet.Tone.ACCENT)
                .message("Appaire le téléphone avec wol-pi, ou renseigne la clé du repli n8n : Réglages › wol-pi.")
                .show { openTab(R.id.nav_settings) }
            return
        }
        val names = nodes.joinToString(", ") { it.pveName }
        ConfirmSheet(act, if (nodes.size > 1) "Tout allumer ?" else "Allumer ${nodes[0].pveName} ?", "⏻ Allumer")
            .kicker("WAKE-ON-LAN · AUDITÉ").tone(ConfirmSheet.Tone.ACCENT)
            .message("Paquet magique envoyé par wol-pi à : $names. Si wol-pi est injoignable, repli automatique par n8n (API Proxmox d'un nœud allumé). Un nœud déjà allumé répond « déjà allumé » (aucun effet). Chaque demande est auditée dans Wazuh.")
            .show { doWake(nodes) }
    }

    private fun doWake(nodes: List<NodeDef>) {
        val ctx = act.applicationContext
        ui.launch {
            val sent = mutableListOf<NodeDef>()
            for (n in nodes) {
                wakeStatus[n.target] = "demande envoyée…"; render()
                val o = WolClient.wakeWithFallback(ctx, n.target)
                val par = when (o.method) {
                    "proxmox-api" -> " (repli : API Proxmox via n8n)"
                    "n8n" -> " (via n8n)"
                    else -> ""
                }
                wakeStatus[n.target] = when (o.result) {
                    "sent" -> "paquet magique envoyé$par, démarrage…"
                    "already_up" -> "déjà allumé"
                    "no_link" -> "pas de lien réseau vers le nœud (${n.via})"
                    "rejected" -> "refusé par wol-pi"
                    "error" -> "échec d'envoi (etherwake)"
                    "fallback_error" -> "wol-pi puis repli Proxmox en échec : ${o.detail.take(80)}"
                    "relay_error" -> "wol-pi : ${o.direct.removePrefix("ssh_error: ").take(60)} · repli : ${o.detail}"
                    else -> (o.detail.ifBlank { o.result }) + par
                }
                if (o.result == "sent") sent += n
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

    private fun note(s: String) = n.text(s, 11f, n.faint).apply { setPadding(n.dp(2), n.dp(2), n.dp(2), n.dp(6)) }

    private fun barRow(name: String, frac: Double, value: String, color: Int) = n.row().apply {
        setPadding(0, n.dp(8), 0, 0)
        addView(n.monoText(name, 10.5f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(n.dp(34), -2) })
        addView(n.bar(frac, color).apply { layoutParams = LinearLayout.LayoutParams(0, n.dp(4), 1f) })
        addView(n.monoText(value, 10.5f, cMuted).apply { layoutParams = LinearLayout.LayoutParams(n.dp(78), -2); gravity = Gravity.END })
    }

    private fun kpi(v: String, l: String, color: Int, last: Boolean = false) = n.tile().apply {
        layoutParams = LinearLayout.LayoutParams(0, -2, 1f).apply { if (!last) marginEnd = n.dp(8) }
        addView(n.title(v, 20f, n.lighten(color)))
        addView(n.text(l, 11f, cMuted))
    }

    /** Message prérempli pour l'agent N1 (JBR) ; modifiable avant envoi. */
    private fun askText(
        level: Int, desc: String, agent: String, t: String,
        id: String = "", ruleId: String = "", srcip: String = "", mitre: List<String> = emptyList(),
    ): String {
        val ctx = buildList {
            if (ruleId.isNotBlank()) add("règle $ruleId")
            add("niveau $level")
            if (agent.isNotBlank()) add("agent $agent")
            if (t.isNotBlank()) add(t)
            if (srcip.isNotBlank()) add("source $srcip")
            if (mitre.isNotEmpty()) add("MITRE ${mitre.joinToString(",")}")
        }.joinToString(", ")
        val ref = if (id.isNotBlank()) "l'alerte Wazuh $id" else "cette alerte Wazuh"
        return "@jbr analyse $ref ($ctx) : « ${desc.take(300)} ». Vrai positif ? Action recommandée ?"
    }

    private fun alertRow(level: Int, t: String, desc: String, sub: String) = LinearLayout(act).apply {
        orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
        val col = levelColorForRule(level)
        background = android.graphics.drawable.GradientDrawable().apply {
            cornerRadius = n.dpf(12f)
            if (level >= 12) { orientation = android.graphics.drawable.GradientDrawable.Orientation.LEFT_RIGHT
                colors = intArrayOf(n.fade(col, 0.12f), ContextCompat.getColor(act, R.color.surface)) }
            else setColor(ContextCompat.getColor(act, R.color.surface))
            setStroke(n.dp(1), if (level >= 12) n.fade(col, 0.4f) else n.border)
        }
        setPadding(n.dp(12), n.dp(10), n.dp(12), n.dp(10))
        layoutParams = n.lp().apply { bottomMargin = n.dp(8) }
        addView(n.levelBadge(level, col).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = n.dp(12) })
        val mid = LinearLayout(act).apply { orientation = LinearLayout.VERTICAL; layoutParams = n.weight() }
        mid.addView(n.ellipsize(n.text(desc, 13f, cText)))
        if (sub.isNotBlank()) mid.addView(n.ellipsize(n.monoText(sub, 11f, cMuted)))
        addView(mid)
        if (t.isNotEmpty()) addView(n.monoText(t, 11f, cMuted).apply { setPadding(n.dp(8), 0, 0, 0) })
    }

    private fun agentRow(name: String, sub: String, ok: Boolean) = n.row().apply {
        setPadding(0, n.dp(6), 0, n.dp(6))
        addView(n.dot(if (ok) cOk else cCrit, 7))
        addView(n.monoText(name, 12.5f, cText).apply { setPadding(n.dp(10), 0, n.dp(8), 0); layoutParams = n.weight() })
        addView(n.monoText(sub, 11f, cMuted))
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
