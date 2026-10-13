package com.jbr.purpleops

import android.os.Bundle
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray

class MainActivity : AppCompatActivity() {

    private val ui = CoroutineScope(Dispatchers.Main + SupervisorJob())

    private lateinit var targetInput: EditText
    private lateinit var cmdInput: EditText
    private lateinit var runButton: Button
    private lateinit var buttonRow: LinearLayout
    private lateinit var stopButton: Button
    private lateinit var clearButton: Button
    private lateinit var console: TextView
    private lateinit var consoleScroll: ScrollView

    // Cible autorisee pour les PRESETS (substitution {T}) : IP, host, CIDR, plages,
    // cibles multiples separees par espace. Les metacaracteres shell sont interdits
    // pour empecher toute evasion de la commande preenregistree.
    private val targetRegex = Regex("^[A-Za-z0-9 ._:/-]{1,200}$")

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        targetInput = findViewById(R.id.targetInput)
        cmdInput = findViewById(R.id.cmdInput)
        runButton = findViewById(R.id.runButton)
        buttonRow = findViewById(R.id.buttonRow)
        stopButton = findViewById(R.id.stopButton)
        clearButton = findViewById(R.id.clearButton)
        console = findViewById(R.id.console)
        consoleScroll = findViewById(R.id.consoleScroll)

        val prefs = getSharedPreferences("purpleops", MODE_PRIVATE)
        targetInput.setText(prefs.getString("target", ""))

        runButton.setOnClickListener { runFree() }
        stopButton.setOnClickListener {
            RootRunner.stop()
            appendLine("[stop demande]")
        }
        clearButton.setOnClickListener { console.text = "" }

        loadPresets()
        appendLine("PurpleOps pret. Perimetre : TES machines uniquement.")
    }

    private fun loadPresets() {
        ui.launch {
            val external = RootRunner.readFileAsRoot("/data/local/soc/presets.json")
            val raw = external ?: assets.open("presets.json").bufferedReader().use { it.readText() }
            val presets = parsePresets(raw)
            if (external != null) appendLine("[presets] charges depuis /data/local/soc/presets.json")
            for (p in presets) addPresetButton(p)
        }
    }

    private fun parsePresets(raw: String): List<Preset> {
        val list = mutableListOf<Preset>()
        try {
            val arr = JSONArray(raw)
            for (i in 0 until arr.length()) {
                val o = arr.getJSONObject(i)
                list.add(
                    Preset(
                        label = o.getString("label"),
                        cmd = o.getString("cmd"),
                        needsTarget = o.optBoolean("needsTarget", true)
                    )
                )
            }
        } catch (e: Exception) {
            appendLine("[erreur presets] ${e.message}")
        }
        return list
    }

    private fun addPresetButton(p: Preset) {
        val b = Button(this)
        b.text = p.label
        b.isAllCaps = false
        val lp = LinearLayout.LayoutParams(
            ViewGroup.LayoutParams.WRAP_CONTENT,
            ViewGroup.LayoutParams.WRAP_CONTENT
        )
        lp.marginEnd = 12
        b.layoutParams = lp
        b.setOnClickListener { runPreset(p) }
        buttonRow.addView(b)
    }

    /** Preset : {T} est substitue et la cible est validee (pas de metacaracteres). */
    private fun runPreset(p: Preset) {
        val target = targetInput.text.toString().trim()
        if (p.needsTarget) {
            if (target.isEmpty()) { toast("Renseigne une cible"); return }
            if (!targetRegex.matches(target)) { toast("Cible invalide (caracteres interdits)"); return }
        }
        saveTarget(target)
        execute(p.cmd.replace("{T}", target))
    }

    /** Commande libre : l'utilisateur tape une commande Kali complete (pipes, etc.).
     *  {T} est substitue par la cible si present, mais aucune restriction de caracteres
     *  (c'est une commande que l'utilisateur ecrit lui-meme, sur son propre appareil). */
    private fun runFree() {
        var cmd = cmdInput.text.toString().trim()
        if (cmd.isEmpty()) { toast("Tape une commande"); return }
        val target = targetInput.text.toString().trim()
        if (cmd.contains("{T}")) {
            if (target.isEmpty()) { toast("Renseigne une cible pour {T}"); return }
            cmd = cmd.replace("{T}", target)
        }
        saveTarget(target)
        execute(cmd)
    }

    private fun execute(inner: String) {
        appendLine("")
        appendLine("\$ $inner")
        stopButton.isEnabled = true
        ui.launch {
            val code = withContext(Dispatchers.IO) {
                RootRunner.runStreaming(inner) { line -> ui.launch { appendLine(line) } }
            }
            appendLine("[termine, code $code]")
            stopButton.isEnabled = false
        }
    }

    private fun saveTarget(target: String) {
        getSharedPreferences("purpleops", MODE_PRIVATE)
            .edit().putString("target", target).apply()
    }

    private fun appendLine(s: String) {
        console.append(s + "\n")
        consoleScroll.post { consoleScroll.fullScroll(ScrollView.FOCUS_DOWN) }
    }

    private fun toast(s: String) = Toast.makeText(this, s, Toast.LENGTH_SHORT).show()

    override fun onDestroy() {
        super.onDestroy()
        RootRunner.stop()
    }
}
