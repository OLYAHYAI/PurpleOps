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
    private lateinit var viewCamera: View
    private lateinit var viewScreen: View
    private lateinit var viewChat: View
    private var chat: ChatPanel? = null
    private var camera: CameraPanel? = null
    private var screen: ScreenPanel? = null
    private var home: HomePanel? = null
    private var radio: RadioPanel? = null
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
    private val neon by lazy { Neon(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        // Secrets hérités en clair (versions <= v28) -> chiffrés par le Keystore.
        try { SecureStore.migrate(applicationContext) } catch (e: Exception) { }

        val contentArea = findViewById<View>(R.id.contentArea)
        bottomNav = findViewById(R.id.bottomNav)
        ViewCompat.setOnApplyWindowInsetsListener(findViewById(R.id.rootLayout)) { _, insets ->
            val sb = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            val ime = insets.getInsets(WindowInsetsCompat.Type.ime())
            contentArea.setPadding(dp(16), sb.top + dp(8), dp(16), dp(6))
            // Barre flottante : la marge (et non le padding) absorbe la barre système,
            // pour que la pastille reste entière au-dessus des gestes.
            (bottomNav.layoutParams as ViewGroup.MarginLayoutParams).let { lp ->
                val m = maxOf(sb.bottom, ime.bottom) + dp(10)
                if (lp.bottomMargin != m) { lp.bottomMargin = m; bottomNav.layoutParams = lp }
            }
            insets
        }

        // La barre M3 ajoute d'elle-même la hauteur de la barre de gestes en
        // padding (doublon avec la marge ci-dessus) : on neutralise ce comportement.
        ViewCompat.setOnApplyWindowInsetsListener(bottomNav) { v, insets ->
            v.setPadding(0, 0, 0, 0)
            insets
        }

        viewHome = findViewById(R.id.viewHome)
        viewAlerts = findViewById(R.id.viewAlerts)
        viewAssets = findViewById(R.id.viewAssets)
        viewKali = findViewById(R.id.viewKali)
        viewSettings = findViewById(R.id.viewSettings)
        viewCamera = findViewById(R.id.viewCamera)
        viewScreen = findViewById(R.id.viewScreen)
        viewChat = findViewById(R.id.viewChat)
        // Une erreur dans la section caméra ne doit jamais empêcher l'appli
        // (et sa mise à jour automatique) de démarrer.
        camera = try { CameraPanel(this, viewCamera, ui) } catch (e: Exception) { null }
        screen = try { ScreenPanel(this, viewScreen, ui) } catch (e: Exception) { null }
        try { McpPanel(this, viewSettings, ui) } catch (e: Exception) { }
        try { WolPanel(this, viewSettings, ui) } catch (e: Exception) { }
        chat = try {
            ChatPanel(this, viewChat, ui, onUnread = { n -> setChatBadge(n) }, audit = { line -> ui.launch { RootRunner.logRun("${sysFmt.format(java.util.Date())} localhost purpleops: cmd=$line") } })
        } catch (e: Exception) { null }
        home = try {
            HomePanel(this, viewHome, ui, { id -> openTab(id) }, { text -> askSoc(text) })
        } catch (e: Exception) { null }

        bindKali()
        bindSettings()

        bottomNav.setOnItemSelectedListener { item -> showTab(item.itemId); true }
        // Depuis Nikon (sans onglet), l'item Accueil reste coché : le re-toucher doit y revenir.
        bottomNav.setOnItemReselectedListener { item -> if (currentTab != item.itemId) showTab(item.itemId) }
        showTab(R.id.nav_home)

        checkRoot()
        requestNotifPermission()
        ui.launch { Updater.checkAndUpdate(applicationContext) { line -> appendLine(line) } }
    }

    /** Onglet de la barre, ou écran sans onglet (Nikon). */
    private fun openTab(id: Int) {
        if (bottomNav.menu.findItem(id) != null) bottomNav.selectedItemId = id else showTab(id)
    }

    /** Ouvre le chat #soc avec un message prérempli (alerte, sortie Kali…). */
    fun askSoc(text: String) {
        chat?.compose(text)
        openTab(R.id.nav_chat)
    }

    /** Pastille sur l'onglet Chat : mentions et décisions arrivées hors de l'écran. */
    private fun setChatBadge(n: Int) {
        if (n <= 0) bottomNav.removeBadge(R.id.nav_chat)
        else bottomNav.getOrCreateBadge(R.id.nav_chat).apply {
            number = n
            backgroundColor = ContextCompat.getColor(this@MainActivity, R.color.danger)
        }
    }

    // Vérification légère du salon (sync sans attente) quand le chat n'est pas affiché,
    // seulement appli au premier plan : alimente la pastille, aucun service en arrière-plan.
    private val chatPeek = object : Runnable {
        override fun run() {
            if (currentTab != R.id.nav_chat) chat?.peek()
            peekHandler.postDelayed(this, 60_000)
        }
    }
    private val peekHandler = android.os.Handler(android.os.Looper.getMainLooper())

    private fun showTab(id: Int) {
        currentTab = id
        viewHome.visibility = if (id == R.id.nav_home) View.VISIBLE else View.GONE
        viewAlerts.visibility = if (id == R.id.nav_alerts) View.VISIBLE else View.GONE
        viewAssets.visibility = if (id == R.id.nav_assets) View.VISIBLE else View.GONE
        viewKali.visibility = if (id == R.id.nav_kali) View.VISIBLE else View.GONE
        viewSettings.visibility = if (id == R.id.nav_settings) View.VISIBLE else View.GONE
        viewCamera.visibility = if (id == R.id.nav_camera) View.VISIBLE else View.GONE
        viewScreen.visibility = if (id == R.id.nav_screen) View.VISIBLE else View.GONE
        viewChat.visibility = if (id == R.id.nav_chat) View.VISIBLE else View.GONE
        chat?.setVisible(id == R.id.nav_chat)
        if (id == R.id.nav_screen) screen?.refreshPcs()
        home?.setVisible(id == R.id.nav_home)
        if (id == R.id.nav_alerts || id == R.id.nav_assets) refreshDashboard()
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

        // Kali › Radio (wol-pi) : écran du capteur WIDS via le webhook n8n,
        // derrière le sélecteur Outils | Radio (plus de boîte de dialogue).
        radio = try { RadioPanel(this, findViewById(R.id.radioView), ui) } catch (e: Exception) { null }
        findViewById<View>(R.id.segTools).setOnClickListener { showOps(radio = false) }
        findViewById<View>(R.id.segRadio).setOnClickListener { showOps(radio = true) }
        showOps(radio = false)

        targetInput.setText(getSharedPreferences("purpleops", MODE_PRIVATE).getString("target", ""))
        runButton.setOnClickListener { runFree() }
        stopButton.setOnClickListener { RootRunner.stop(); appendLine("[stop demande]") }
        clearButton.setOnClickListener { console.text = "" }
        // → #soc : soumettre la dernière commande et sa sortie à un agent (message modifiable avant envoi).
        findViewById<View>(R.id.askSocButton).setOnClickListener { askSoc(kaliExcerpt()) }
        backBtn.setOnClickListener { goUp() }

        appendLine("PurpleOps v${BuildConfig.VERSION_NAME} (code ${BuildConfig.VERSION_CODE})")
        appendLine("Perimetre : TES machines uniquement.")
        loadCatalog()
    }

    /** Sous-onglet de Kali : Outils (catalogue + console) ou Radio. */
    private fun showOps(radio: Boolean) {
        findViewById<View>(R.id.segTools).isSelected = !radio
        findViewById<View>(R.id.segRadio).isSelected = radio
        findViewById<View>(R.id.kaliTools).visibility = if (radio) View.GONE else View.VISIBLE
        findViewById<View>(R.id.radioView).visibility = if (radio) View.VISIBLE else View.GONE
        findViewById<TextView>(R.id.opsKicker).text = if (radio) "OPÉRATIONS · WIDS WOL-PI" else "OPÉRATIONS · CHROOT KALI"
        findViewById<TextView>(R.id.opsTitle).text = if (radio) "Radio" else "Kali"
        findViewById<View>(R.id.rootChip).visibility = if (radio) View.GONE else View.VISIBLE
        if (radio) this.radio?.onShown()
    }

    private fun checkRoot() {
        ui.launch {
            val (_, out) = RootRunner.runAndroidRoot("id")
            val ok = out.contains("uid=0")
            rootChip.text = if (ok) "ROOT OK" else "ROOT ✗"
            rootChip.setBackgroundResource(if (ok) R.drawable.chip_ok else R.drawable.chip_danger)
            rootChip.setTextColor(ContextCompat.getColor(this@MainActivity, if (ok) R.color.ok_soft else R.color.danger_soft))
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
                categories.forEachIndexed { i, c ->
                    addRow(c.name, c.tools.joinToString(" · ") { it.name }, String.format("%02d", i + 1)) {
                        curCat = c; level = Level.TOOLS; render()
                    }
                }
            }
            Level.TOOLS -> {
                breadcrumb.text = "Catalogue › ${curCat?.name}"; backBtn.visibility = View.VISIBLE
                curCat!!.tools.forEachIndexed { i, t -> addRow(t.name, t.desc, String.format("%02d", i + 1)) { curTool = t; level = Level.ACTIONS; render() } }
            }
            Level.ACTIONS -> {
                breadcrumb.text = "Catalogue › ${curCat?.name} › ${curTool?.name}"; backBtn.visibility = View.VISIBLE
                for (a in curTool!!.actions) addRow(a.label, a.cmd, "▶") { onAction(a) }
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

    /** Ligne du catalogue Kali : pastille d'index, titre, sous-titre mono, chevron. */
    private fun addRow(title: String, subtitle: String?, badge: String, onClick: () -> Unit) {
        val n = neon
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
            setBackgroundResource(R.drawable.btn_tile)
            setPadding(dp(12), dp(10), dp(14), dp(10))
            minimumHeight = dp(56)
            isClickable = true; isFocusable = true
            setOnClickListener { onClick() }
        }
        row.layoutParams = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT
        ).apply { bottomMargin = dp(6) }
        row.addView(n.monoText(badge, 11f, n.accentSoft, bold = true).apply {
            gravity = Gravity.CENTER
            background = android.graphics.drawable.GradientDrawable().apply { cornerRadius = n.dpf(9f); setColor(n.fade(n.accent, 0.14f)) }
            layoutParams = LinearLayout.LayoutParams(dp(32), dp(32)).apply { marginEnd = dp(12) }
        })
        val col = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; layoutParams = LinearLayout.LayoutParams(0, -2, 1f) }
        col.addView(n.text(title, 14.5f, n.text).apply { setTypeface(typeface, Typeface.BOLD) })
        if (!subtitle.isNullOrBlank()) col.addView(n.ellipsize(n.monoText(subtitle, 11f, n.muted)))
        row.addView(col)
        row.addView(n.text("›", 18f, n.muted).apply { setPadding(dp(8), 0, 0, 0) })
        contentList.addView(row)
    }

    /** Paramètres d'une action du catalogue : même feuille que les autres confirmations. */
    private fun onAction(a: ToolAction) {
        if (a.params.isEmpty()) { buildAndRun(a.cmd, emptyMap()); return }
        val sheet = ConfirmSheet(this, a.label, "RUN")
            .kicker("${curTool?.name ?: "KALI"} · ${a.cmd.take(40)}")
            .tone(ConfirmSheet.Tone.ACCENT)
        for (p in a.params) sheet.field(p.name, p.hint, p.hint, initial = p.def)
        sheet.show { values -> buildAndRun(a.cmd, values) }
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

    private fun kaliExcerpt(): String {
        val lines = console.text.toString().trimEnd().lines()
        val start = lines.indexOfLast { it.startsWith("$ ") }.coerceAtLeast(0)
        val tail = lines.drop(start).takeLast(25).joinToString("\n").take(2500)
        return "@jbr résultat d'une commande PurpleOps (Kali, lab) à analyser :\n$tail"
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
        ui.launch {
            val res = WazuhClient.load(applicationContext)
            dashLoading = false
            val allAlerts = findViewById<LinearLayout>(R.id.alertsList).also { it.removeAllViews() }
            val allAssets = findViewById<LinearLayout>(R.id.assetsList).also { it.removeAllViews() }
            when (res) {
                is WzResult.NotConfigured -> { addInfo(allAlerts, "Non configuré — onglet Réglages"); addInfo(allAssets, "Non configuré — onglet Réglages") }
                is WzResult.Err -> { addInfo(allAlerts, "Wazuh injoignable : ${res.message}"); addInfo(allAssets, "Wazuh injoignable : ${res.message}") }
                is WzResult.Ok -> {
                    val s = res.summary
                    if (s.alerts.isEmpty()) addInfo(allAlerts, "Aucune alerte sur 24h")
                    for (a in s.alerts) {
                        val dot = if (a.level >= 10) "#FF4D5E" else if (a.level >= 7) "#F5A524" else "#4DA3FF"
                        addListItem(allAlerts, dot, a.desc, "${a.agent} · niveau ${a.level}")
                    }
                    for (ag in s.agents) {
                        val dot = if (ag.status.equals("active", true)) "#2ECC71" else "#FF4D5E"
                        addListItem(allAssets, dot, ag.name, "${ag.status} · ${ag.ip}")
                    }
                }
            }
        }
    }

    private fun addInfo(parent: LinearLayout, text: String) {
        parent.addView(TextView(this).apply {
            this.text = text; setTextColor(Color.parseColor("#8B98A6")); textSize = 13f
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
        col.addView(TextView(this).apply { text = title; setTextColor(Color.parseColor("#E6EDF3")); textSize = 14f; setTypeface(null, Typeface.BOLD) })
        col.addView(TextView(this).apply { text = subtitle; setTextColor(Color.parseColor("#8B98A6")); textSize = 11f })
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
        apiPass.setText(SecureStore.get(this, "wz_api_pass"))
        idxUser.setText(p.getString("wz_idx_user", ""))
        idxPass.setText(SecureStore.get(this, "wz_idx_pass"))
        version.text = "PurpleOps v${BuildConfig.VERSION_NAME} (code ${BuildConfig.VERSION_CODE})"

        // Webhook n8n de l'écran Radio (déplacé depuis l'ancienne boîte de dialogue).
        val radioUrl = findViewById<EditText>(R.id.cfgRadioUrl)
        radioUrl.setText(RadioClient.url(applicationContext))
        findViewById<View>(R.id.cfgRadioSave).setOnClickListener {
            RadioClient.setUrl(applicationContext, radioUrl.text.toString())
            radioUrl.setText(RadioClient.url(applicationContext))
            toast("URL du webhook radio enregistrée")
        }

        save.setOnClickListener {
            val newHost = host.text.toString().trim()
            val ed = p.edit()
            // Nouvel hôte = nouvelles empreintes à valider.
            if (newHost != p.getString("wz_host", "")) ed.remove(WazuhClient.PIN_API).remove(WazuhClient.PIN_IDX)
            ed.putString("wz_host", newHost)
                .putString("wz_api_user", apiUser.text.toString().trim())
                .putString("wz_idx_user", idxUser.text.toString().trim())
            SecureStore.put(ed, "wz_api_pass", apiPass.text.toString())
            SecureStore.put(ed, "wz_idx_pass", idxPass.text.toString())
            ed.apply()
            status.text = "Enregistré (mots de passe chiffrés). Puis « Tester » pour valider les certificats."
            toast("Réglages enregistrés")
        }
        test.setOnClickListener {
            status.text = "Test en cours…"
            ui.launch {
                val h = p.getString("wz_host", "").orEmpty()
                val needPins = p.getString(WazuhClient.PIN_API, "").isNullOrEmpty() ||
                    (p.getString("wz_idx_user", "").orEmpty().isNotEmpty() && p.getString(WazuhClient.PIN_IDX, "").isNullOrEmpty())
                if (h.isNotEmpty() && needPins) {
                    if (!confirmPins(h)) { status.text = "Empreintes non validées : connexion refusée."; return@launch }
                }
                when (val res = WazuhClient.load(applicationContext)) {
                    is WzResult.NotConfigured -> status.text = "Renseigne au moins l'hôte + identifiants API, puis Enregistrer."
                    is WzResult.Err -> {
                        status.text = "Échec : ${res.message}"
                        if (res.message.contains("CHANGÉ") && confirmPins(h)) status.append("\nNouvelles empreintes enregistrées : relance « Tester ».")
                    }
                    is WzResult.Ok -> status.text = "OK — ${res.summary.total} agents (${res.summary.active} actifs), ${res.summary.alerts24h} alertes/24h."
                }
            }
        }
        checkUpdate.setOnClickListener {
            status.text = "Recherche de mise a jour…"
            ui.launch { Updater.checkAndUpdate(applicationContext) { line -> status.append("\n$line") } }
        }
    }

    /**
     * Lit les clés publiques présentées par l'API (55000) et l'Indexer (9200),
     * affiche leurs empreintes SPKI SHA-256 et ne les enregistre qu'après accord
     * explicite. À comparer avec, sur le manager :
     *   openssl s_client -connect 127.0.0.1:55000 </dev/null | openssl x509 -pubkey -noout \
     *     | openssl pkey -pubin -outform der | openssl dgst -sha256 -binary | base64
     */
    private suspend fun confirmPins(host: String): Boolean {
        val probes = withContext(Dispatchers.IO) {
            listOf(WazuhClient.API_PORT, WazuhClient.IDX_PORT).associateWith { port ->
                try { PinnedTls.probe(host, port) } catch (e: Exception) { null }
            }
        }
        val api = probes[WazuhClient.API_PORT] ?: run { toast("API $host:55000 injoignable"); return false }
        val idx = probes[WazuhClient.IDX_PORT]
        val msg = buildString {
            append("Vérifie ces empreintes (SPKI SHA-256) sur le manager avant d'accepter.\n\n")
            append("API 55000\n${api.first}\n\n")
            append(if (idx != null) "Indexer 9200\n${idx.first}" else "Indexer 9200 : injoignable (non épinglé)")
        }
        return kotlinx.coroutines.suspendCancellableCoroutine { cont ->
            val d = AlertDialog.Builder(this)
                .setTitle("Certificats de $host")
                .setMessage(msg)
                .setPositiveButton("Faire confiance") { _, _ ->
                    val ed = getSharedPreferences("purpleops", MODE_PRIVATE).edit().putString(WazuhClient.PIN_API, api.first)
                    if (idx != null) ed.putString(WazuhClient.PIN_IDX, idx.first)
                    ed.apply()
                    if (cont.isActive) cont.resumeWith(Result.success(true))
                }
                .setNegativeButton("Refuser") { _, _ -> if (cont.isActive) cont.resumeWith(Result.success(false)) }
                .setOnCancelListener { if (cont.isActive) cont.resumeWith(Result.success(false)) }
                .show()
            d.findViewById<TextView>(android.R.id.message)?.setTextIsSelectable(true)
            cont.invokeOnCancellation { d.dismiss() }
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

    override fun onResume() {
        super.onResume()
        home?.setVisible(currentTab == R.id.nav_home)
        chat?.setVisible(currentTab == R.id.nav_chat)
        peekHandler.removeCallbacks(chatPeek)
        peekHandler.postDelayed(chatPeek, 3_000)
    }

    override fun onPause() {
        home?.setVisible(false)   // pas de rafraîchissement en arrière-plan
        chat?.setVisible(false)   // pas de /sync en arrière-plan
        peekHandler.removeCallbacks(chatPeek)
        super.onPause()
    }

    override fun onDestroy() {
        super.onDestroy()
        RootRunner.stop()
        camera?.stop()
        screen?.onAppDestroyed()
    }
}
