package com.jbr.purpleops

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.Gravity
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
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class MainActivity : AppCompatActivity() {

    private val ui = CoroutineScope(Dispatchers.Main + SupervisorJob())

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

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

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

        val prefs = getSharedPreferences("purpleops", MODE_PRIVATE)
        targetInput.setText(prefs.getString("target", ""))

        runButton.setOnClickListener { runFree() }
        stopButton.setOnClickListener { RootRunner.stop(); appendLine("[stop demande]") }
        clearButton.setOnClickListener { console.text = "" }
        backBtn.setOnClickListener { goUp() }

        appendLine("PurpleOps v${BuildConfig.VERSION_NAME} (code ${BuildConfig.VERSION_CODE})")
        appendLine("Perimetre : TES machines uniquement.")

        loadCatalog()
        checkRoot()
        requestNotifPermission()
        ui.launch { Updater.checkAndUpdate(applicationContext) { line -> appendLine(line) } }
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
            categories = try {
                Catalog.parse(raw)
            } catch (e: Exception) {
                appendLine("[erreur catalogue] ${e.message}"); emptyList()
            }
            level = Level.CATEGORIES
            render()
        }
    }

    private fun render() {
        contentList.removeAllViews()
        when (level) {
            Level.CATEGORIES -> {
                breadcrumb.text = "Catalogue"
                backBtn.visibility = TextView.GONE
                for (c in categories) addRow("▸  ${c.name}", "${c.tools.size} outil(s)") {
                    curCat = c; level = Level.TOOLS; render()
                }
            }
            Level.TOOLS -> {
                breadcrumb.text = "Catalogue › ${curCat?.name}"
                backBtn.visibility = TextView.VISIBLE
                for (t in curCat!!.tools) addRow(t.name, t.desc) {
                    curTool = t; level = Level.ACTIONS; render()
                }
            }
            Level.ACTIONS -> {
                breadcrumb.text = "Catalogue › ${curCat?.name} › ${curTool?.name}"
                backBtn.visibility = TextView.VISIBLE
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
        if (level != Level.CATEGORIES) goUp() else super.onBackPressed()
    }

    private fun addRow(title: String, subtitle: String?, onClick: () -> Unit) {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundResource(R.drawable.btn_neon)
            setPadding(dp(14), dp(12), dp(14), dp(12))
            isClickable = true
            setOnClickListener { onClick() }
        }
        val lp = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT
        ).apply { bottomMargin = dp(8) }
        row.layoutParams = lp

        row.addView(TextView(this).apply {
            text = title
            setTextColor(Color.parseColor("#EDE9F5"))
            textSize = 15f
        })
        if (!subtitle.isNullOrBlank()) row.addView(TextView(this).apply {
            text = subtitle
            setTextColor(Color.parseColor("#8A7FA5"))
            textSize = 11f
            typeface = android.graphics.Typeface.MONOSPACE
        })
        contentList.addView(row)
    }

    private fun onAction(a: ToolAction) {
        if (a.params.isEmpty()) {
            buildAndRun(a.cmd, emptyMap()); return
        }
        // Dialogue de parametres.
        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(8), dp(20), dp(0))
        }
        val inputs = HashMap<String, EditText>()
        for (p in a.params) {
            container.addView(TextView(this).apply {
                text = p.hint
                setTextColor(Color.parseColor("#8A7FA5"))
                textSize = 12f
            })
            val et = EditText(this).apply {
                setText(p.def)
                setTextColor(Color.parseColor("#EDE9F5"))
                setHintTextColor(Color.parseColor("#8A7FA5"))
                setSingleLine()
            }
            inputs[p.name] = et
            container.addView(et)
        }
        AlertDialog.Builder(this)
            .setTitle(a.label)
            .setView(container)
            .setPositiveButton("RUN") { _, _ ->
                buildAndRun(a.cmd, inputs.mapValues { it.value.text.toString().trim() })
            }
            .setNegativeButton("Annuler", null)
            .show()
    }

    private fun buildAndRun(template: String, params: Map<String, String>) {
        var cmd = template
        val target = targetInput.text.toString().trim()
        if (cmd.contains("{T}")) {
            if (target.isEmpty()) { toast("Renseigne une cible"); return }
            if (!targetRegex.matches(target)) { toast("Cible invalide"); return }
            cmd = cmd.replace("{T}", target)
            saveTarget(target)
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

    private fun execute(inner: String) {
        appendLine("")
        appendLine("$ $inner")
        ui.launch {
            val code = withContext(Dispatchers.IO) {
                RootRunner.runStreaming(inner) { line -> ui.launch { appendLine(line) } }
            }
            appendLine("[termine, code $code]")
        }
    }

    private fun saveTarget(target: String) {
        getSharedPreferences("purpleops", MODE_PRIVATE).edit()
            .putString("target", target).apply()
    }

    private fun appendLine(s: String) {
        console.append(s + "\n")
        consoleScroll.post { consoleScroll.fullScroll(ScrollView.FOCUS_DOWN) }
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
