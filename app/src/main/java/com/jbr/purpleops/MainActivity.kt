package com.jbr.purpleops

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
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
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import com.google.android.material.bottomnavigation.BottomNavigationView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : AppCompatActivity() {

    private val ui = CoroutineScope(Dispatchers.Main + SupervisorJob())

    // Vues des onglets
    private lateinit var viewHome: View
    private lateinit var viewAlerts: View
    private lateinit var viewAssets: View
    private lateinit var viewKali: View
    private lateinit var viewSettings: View
    private lateinit var bottomNav: BottomNavigationView
    private var currentTab = R.id.nav_home

    // Kali
    private lateinit var targetInput: EditText
    private lateinit var cmdInput: EditText
    private lateinit var runButton: TextView
    private lateinit var stopButton: TextView
    private lateinit var clearButton: TextView
    private lateinit var backBtn: TextView
    private lateinit var breadcrumb: TextView
    private lateinit var contentList: LinearLayout
    private lateinit var console: TextView
    private lateinit var consoleScroll: ScrollView
    private lateinit var rootChip: TextView

    private var categories: List<Category> = emptyList()
    private enum class Level { CATEGORIES, TOOLS, ACTIONS }
    private var level = Level.CATEGORIES
    private var curCat: Category? = null
    private var curTool: Tool? = null

    private val targetRegex = Regex("^[A-Za-z0-9 ._:/-]{1,200}$")
    private val ansi = Regex("\\u001B\\[[0-9;]*[a-zA-Z]")

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        val contentArea = findViewById<View>(R.id.contentArea)
        bottomNav = findViewById(R.id.bottomNav)
        ViewCompat.setOnApplyWindowInsetsListener(findViewById(R.id.rootLayout)) { _, insets ->
            val sb = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
            contentArea.setPadding(dp(12), sb.top + dp(6), dp(12), dp(6))
            bottomNav.setPadding(0, 0, 0, maxOf(sb.bottom, ime.bottom))
            insets
        }

        viewHome = findViewById(R.id.viewHome)
        viewAlerts = findViewById(R.id.viewAlerts)
        viewAssets = findViewById(R.id.viewAssets)
        viewKali = findViewById(R.id.viewKali)
        viewSettings = findViewById(R.id.viewSettings)

        bindKali()
        bindSettings()

        bottomNav.setOnItemSelectedListener { item -> showTab(item.itemId); true }
        showTab(R.id.nav_home)

        checkRoot()
        requestNotifPermission()
        ui.launch { Updater.checkAndUpdate(applicationContext) { line -> appendLine(line) } }
    }

    private fun showTab(id: Int) {
        currentTab = id
        viewHome.visibility = if (id == R.id.nav_home) View.VISIBLE else View.GONE
        viewAlerts.visibility = if (id == R.id.nav_alerts) View.VISIBLE else View.GONE
        viewAssets.visibility = if (id == R.id.nav_assets) View.VISIBLE else View.GONE
        viewKali.visibility = if (id == R.id.nav_kali) View.VISIBLE else View.GONE
        viewSettings.visibility = if (id == R.id.nav_settings) View.VISIBLE else View.GONE
        if (id == R.id.nav_home || id == R.id.nav_alerts || id == R.id.nav_assets) refreshDashboard()
    }

    // ---------------- KALI ----------------

    private fun bindKali() {
        targetInput = findViewById(R.id.targetInput)
        cmdInput = findViewById(R.id.cmdInput)
        runButton = findViewById(R.id.runButton)
        stopButton = findViewById(R.id.stopButton)
        clearButton = findViewById(R.id.clearButton)
        backBtn = findViewById(R.id.backBtn)
        breadcrumb = findViewById(R.id.breadcrumb)
        contentList = findViewById(R.id.contentList)
        console = findViewById(R.id.console)
        consoleScroll = findViewById(R.id.consoleScroll)
        rootChip = findViewById(R.id.rootChip)

        targetInput.setText(getSharedPreferences("purpleops", MODE_PRIVATE).getString("target", ""))
        runButton.setOnClickListener { runFree() }
        stopButton.setOnClickListener { RootRunner.stop(); appendLine("[stop demande]") }
        clearButton.setOnClickListener { console.text = "" }
        backBtn.setOnClickListener { goUp() }

        appendLine("PurpleOps v${BuildConfig.VERSION_NAME} (code ${BuildConfig.VERSION_CODE})")
        appendLine("Perimetre : TES machines uniquement.")
        loadCatalog()
    }

    private fun checkRoot() {
        ui.launch {
            val (_, out) = RootRunner.runAndroidRoot("id")
            val ok = out.contains("uid=0")
            rootChip.text = if (ok) "root ✓" else "root ✗"
            rootChip.setTextColor(Color.parseColor(if (ok) "#5CFFA6" else "#FF5C7A"))
        }
    }

    private fun loadCatalog() {
        ui.launch {
            val external = RootRunner.readFileAsRoot("/data/local/soc/catalog.json")
            val raw = external ?: assets.open("catalog.json").bufferedReader().use { it.readText() }
            if (external != null) appendLine("[catalogue] /data/local/soc/catalog.json")
            categories = try { Catalog.parse(raw) } catch (e: Exception) {
                appendLine("[erreur catalogue] ${e.message}"); emptyList()
            }
            level = Level.CATEGORIES; render()
        }
    }

    private fun render() {
        contentList.removeAllViews()
        when (level) {
            Level.CATEGORIES -> {
                breadcrumb.text = "Catalogue"; backBtn.visibility = View.GONE
                for (c in categories) addRow("▸  ${c.name}", "${c.tools.size} outil(s)") {
                    curCat = c; level = Level.TOOLS; render()
                }
            }
            Level.TOOLS -> {
                breadcrumb.text = "Catalogue › ${curCat?.name}"; backBtn.visibility = View.VISIBLE
                for (t in curCat!!.tools) addRow(t.name, t.desc) { curTool = t; level = Level.ACTIONS; render() }
            }
            Level.ACTIONS -> {
                breadcrumb.text = "Catalogue › ${curCat?.name} › ${curTool?.name}"; backBtn.visibility = View.VISIBLE
                for (a in curTool!!.actions) addRow(a.label, a.cmd) { onAction(a) }
            }
        }
    }

    private fun goUp() {
        when (level) {
            Level.ACTIONS -> { level = Level.TOOLS; render() }
            Level.TOOLS -> { level = Level.CATEGORIES; render() }
            Level.CATEGORIES -> {}
        }
    }

    override fun onBackPressed() {
        if (currentTab == R.id.nav_kali && level != Level.CATEGORIES) goUp()
        else if (currentTab != R.id.nav_home) bottomNav.selectedItemId = R.id.nav_home
        else super.onBackPressed()
    }

    private fun addRow(title: String, subtitle: String?, onClick: () -> Unit) {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundResource(R.drawable.btn_neon)
            setPadding(dp(14), dp(12), dp(14), dp(12))
            isClickable = true
            setOnClickListener { onClick() }
        }
        row.layoutParams = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT
        ).apply { bottomMargin = dp(8) }
        row.addView(TextView(this).apply { text = title; setTextColor(Color.parseColor("#EDE9F5")); textSize = 15f })
        if (!subtitle.isNullOrBlank()) row.addView(TextView(this).apply {
            text = subtitle; setTextColor(Color.parseColor("#8A7FA5")); textSize = 11f; typeface = Typeface.MONOSPACE
        })
        contentList.addView(row)
    }

    private fun onAction(a: ToolAction) {
        if (a.params.isEmpty()) { buildAndRun(a.cmd, emptyMap()); return }
        val container = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(dp(20), dp(8), dp(20), 0) }
        val inputs = HashMap<String, EditText>()
        for (p in a.params) {
            container.addView(TextView(this).apply { text = p.hint; setTextColor(Color.parseColor("#8A7FA5")); textSize = 12f })
            val et = EditText(this).apply {
                setText(p.def); setTextColor(Color.parseColor("#EDE9F5")); setHintTextColor(Color.parseColor("#8A7FA5")); setSingleLine()
            }
            inputs[p.name] = et; container.addView(et)
        }
        AlertDialog.Builder(this).setTitle(a.label).setView(container)
            .setPositiveButton("RUN") { _, _ -> buildAndRun(a.cmd, inputs.mapValues { it.value.text.toString().trim() }) }
            .setNegativeButton("Annuler", null).show()
    }

    private fun buildAndRun(template: String, params: Map<String, String>) {
        var cmd = template
        val target = targetInput.text.toString().trim()
        if (cmd.contains("{T}")) {
            if (target.isEmpty()) { toast("Renseigne une cible"); return }
            if (!targetRegex.matches(target)) { toast("Cible invalide"); return }
            cmd = cmd.replace("{T}", target); saveTarget(target)
        }
        for ((k, v) in params) cmd = cmd.replace("{$k}", v)
        execute(cmd)
    }

    private fun runFree() {
        var cmd = cmdInput.text.toString().trim()
        if (cmd.isEmpty()) { toast("Tape une commande"); return }
        val target = targetInput.text.toString().trim()
        if (cmd.contains("{T}")) {
            if (target.isEmpty()) { toast("Renseigne une cible pour {T}"); return }
            cmd = cmd.replace("{T}", target); saveTarget(target)
        }
        execute(cmd)
    }

    private val sysFmt = java.text.SimpleDateFormat("MMM dd HH:mm:ss", java.util.Locale.US)

    private fun execute(inner: String) {
        appendLine(""); appendLine("$ $inner")
        // Journal des runs -> lu par l'agent Wazuh (trace "quelle commande").
        val logLine = "${sysFmt.format(java.util.Date())} localhost purpleops: cmd=$inner"
        ui.launch { RootRunner.logRun(logLine) }
        ui.launch {
            val code = withContext(Dispatchers.IO) {
                RootRunner.runStreaming(inner) { line -> ui.launch { appendLine(line) } }
            }
            appendLine("[termine, code $code]")
        }
    }

    private fun saveTarget(target: String) {
        getSharedPreferences("purpleops", MODE_PRIVATE).edit().putString("target", target).apply()
    }

    private fun appendLine(s: String) {
        console.append(ansi.replace(s, "") + "\n")
        consoleScroll.post { consoleScroll.fullScroll(ScrollView.FOCUS_DOWN) }
    }

    // ---------------- DASHBOARD (demo en V3.0, Wazuh en V3.1) ----------------

    private var dashLoading = false

    private fun refreshDashboard() {
        if (dashLoading) return
        dashLoading = true
        findViewById<TextView>(R.id.homeStatusSub).text = "Chargement Wazuh…"
        ui.launch {
            val res = WazuhClient.load(applicationContext)
            dashLoading = false
            when (res) {
                is WzResult.NotConfigured -> showDemo("Non configuré — onglet Réglages")
                is WzResult.Err -> showDemo("Wazuh injoignable : ${res.message}")
                is WzResult.Ok -> showLive(res.summary)
            }
        }
    }

    private fun colorFor(label: String) = when (label) {
        "Protégé" -> "#5CFFA6"; "Attention" -> "#FFB454"; else -> "#FF5C7A"
    }

    private fun showLive(s: WzSummary) {
        findViewById<TextView>(R.id.demoTag).visibility = View.GONE
        val col = colorFor(s.statusLabel)
        findViewById<TextView>(R.id.homeStatus).apply { text = s.statusLabel; setTextColor(Color.parseColor(col)) }
        findViewById<TextView>(R.id.homeStatusSub).text = "Mis à jour à l'instant"
        findViewById<TextView>(R.id.homeScore).apply { text = s.score.toString(); setTextColor(Color.parseColor(col)) }
        findViewById<TextView>(R.id.statAgents).text = s.total.toString()
        findViewById<TextView>(R.id.statAlerts).text = s.alerts24h.toString()
        val pct = if (s.total > 0) s.active * 100 / s.total else 0
        findViewById<TextView>(R.id.statUptime).text = "$pct%"

        val homeAlerts = findViewById<LinearLayout>(R.id.homeAlerts).also { it.removeAllViews() }
        val allAlerts = findViewById<LinearLayout>(R.id.alertsList).also { it.removeAllViews() }
        if (s.alerts.isEmpty()) addInfo(homeAlerts, "Aucune alerte sur 24h")
        for ((i, a) in s.alerts.withIndex()) {
            val dot = if (a.level >= 10) "#FF5C7A" else if (a.level >= 7) "#FFB454" else "#5CF2FF"
            val sub = "${a.agent} · niveau ${a.level}"
            if (i < 5) addListItem(homeAlerts, dot, a.desc, sub)
            addListItem(allAlerts, dot, a.desc, sub)
        }

        val homeAssets = findViewById<LinearLayout>(R.id.homeAssets).also { it.removeAllViews() }
        val allAssets = findViewById<LinearLayout>(R.id.assetsList).also { it.removeAllViews() }
        if (s.agents.isEmpty()) addInfo(homeAssets, "Aucun agent")
        for ((i, ag) in s.agents.withIndex()) {
            val dot = if (ag.status.equals("active", true)) "#5CFFA6" else "#FF5C7A"
            val sub = "${ag.status} · ${ag.ip}"
            if (i < 6) addListItem(homeAssets, dot, ag.name, sub)
            addListItem(allAssets, dot, ag.name, sub)
        }
    }

    private fun showDemo(reason: String) {
        findViewById<TextView>(R.id.demoTag).visibility = View.VISIBLE
        findViewById<TextView>(R.id.homeStatus).apply { text = "Protégé"; setTextColor(Color.parseColor("#EDE9F5")) }
        findViewById<TextView>(R.id.homeStatusSub).text = reason
        findViewById<TextView>(R.id.homeScore).apply { text = "92"; setTextColor(Color.parseColor("#B388FF")) }
        findViewById<TextView>(R.id.statAgents).text = "6"
        findViewById<TextView>(R.id.statAlerts).text = "2"
        findViewById<TextView>(R.id.statUptime).text = "99%"
        val ha = findViewById<LinearLayout>(R.id.homeAlerts).also { it.removeAllViews() }
        addListItem(ha, "#FF5C7A", "Connexion inhabituelle", "demo")
        addListItem(ha, "#FFB454", "Certificat SSL expire bientot", "demo")
        val aa = findViewById<LinearLayout>(R.id.alertsList).also { it.removeAllViews() }
        addListItem(aa, "#FF5C7A", "Connexion inhabituelle", "demo")
        addListItem(aa, "#FFB454", "Certificat SSL expire bientot", "demo")
        val hs = findViewById<LinearLayout>(R.id.homeAssets).also { it.removeAllViews() }
        addListItem(hs, "#5CFFA6", "Serveur de production", "demo")
        addListItem(hs, "#FFB454", "Point d'acces Wi-Fi", "demo")
        val az = findViewById<LinearLayout>(R.id.assetsList).also { it.removeAllViews() }
        addListItem(az, "#5CFFA6", "Serveur de production", "demo")
        addListItem(az, "#FFB454", "Point d'acces Wi-Fi", "demo")
    }

    private fun addInfo(parent: LinearLayout, text: String) {
        parent.addView(TextView(this).apply {
            this.text = text; setTextColor(Color.parseColor("#8A7FA5")); textSize = 13f
            setPadding(dp(4), dp(8), dp(4), dp(8))
        })
    }

    private fun addListItem(parent: LinearLayout, dotColor: String, title: String, subtitle: String) {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundResource(R.drawable.card)
            setPadding(dp(14), dp(12), dp(14), dp(12))
        }
        row.layoutParams = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT
        ).apply { bottomMargin = dp(8) }

        val dot = View(this).apply { setBackgroundColor(Color.parseColor(dotColor)) }
        dot.layoutParams = LinearLayout.LayoutParams(dp(10), dp(10)).apply { rightMargin = dp(12) }
        row.addView(dot)

        val col = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        col.layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
        col.addView(TextView(this).apply { text = title; setTextColor(Color.parseColor("#EDE9F5")); textSize = 14f; setTypeface(null, Typeface.BOLD) })
        col.addView(TextView(this).apply { text = subtitle; setTextColor(Color.parseColor("#8A7FA5")); textSize = 11f })
        row.addView(col)
        parent.addView(row)
    }

    // ---------------- REGLAGES ----------------

    private fun bindSettings() {
        val host = findViewById<EditText>(R.id.cfgHost)
        val apiUser = findViewById<EditText>(R.id.cfgApiUser)
        val apiPass = findViewById<EditText>(R.id.cfgApiPass)
        val idxUser = findViewById<EditText>(R.id.cfgIdxUser)
        val idxPass = findViewById<EditText>(R.id.cfgIdxPass)
        val status = findViewById<TextView>(R.id.cfgStatus)
        val version = findViewById<TextView>(R.id.cfgVersion)
        val save = findViewById<TextView>(R.id.cfgSave)
        val test = findViewById<TextView>(R.id.cfgTest)
        val checkUpdate = findViewById<TextView>(R.id.cfgCheckUpdate)

        val p = getSharedPreferences("purpleops", MODE_PRIVATE)
        host.setText(p.getString("wz_host", ""))
        apiUser.setText(p.getString("wz_api_user", ""))
        apiPass.setText(p.getString("wz_api_pass", ""))
        idxUser.setText(p.getString("wz_idx_user", ""))
        idxPass.setText(p.getString("wz_idx_pass", ""))
        version.text = "PurpleOps v${BuildConfig.VERSION_NAME} (code ${BuildConfig.VERSION_CODE})"

        save.setOnClickListener {
            p.edit()
                .putString("wz_host", host.text.toString().trim())
                .putString("wz_api_user", apiUser.text.toString().trim())
                .putString("wz_api_pass", apiPass.text.toString())
                .putString("wz_idx_user", idxUser.text.toString().trim())
                .putString("wz_idx_pass", idxPass.text.toString())
                .apply()
            status.text = "Enregistré. Le dashboard se rafraîchira."
            toast("Réglages enregistrés")
        }
        test.setOnClickListener {
            status.text = "Test en cours…"
            ui.launch {
                when (val res = WazuhClient.load(applicationContext)) {
                    is WzResult.NotConfigured -> status.text = "Renseigne au moins l'hôte + identifiants API, puis Enregistrer."
                    is WzResult.Err -> status.text = "Échec : ${res.message}"
                    is WzResult.Ok -> status.text = "OK — ${res.summary.total} agents (${res.summary.active} actifs), ${res.summary.alerts24h} alertes/24h."
                }
            }
        }
        checkUpdate.setOnClickListener {
            status.text = "Recherche de mise a jour…"
            ui.launch { Updater.checkAndUpdate(applicationContext) { line -> status.append("\n$line") } }
        }
    }

    private fun toast(s: String) = Toast.makeText(this, s, Toast.LENGTH_SHORT).show()

    private fun requestNotifPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        RootRunner.stop()
    }
}
