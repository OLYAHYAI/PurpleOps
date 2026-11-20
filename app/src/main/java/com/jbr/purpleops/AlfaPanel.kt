package com.jbr.purpleops

import android.graphics.Typeface
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.EditText
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Écran Kali › Alfa : pilote la carte Wi-Fi USB **locale** du téléphone
 * (Alfa AWUS036NH, Ralink RT3070, USB 148f:3070) branchée en OTG. À ne pas
 * confondre avec l'écran Radio, qui parle au capteur WIDS distant (wol-pi).
 *
 * Pensé « tap-and-go » pour la soutenance : un seul bouton **Armer** charge la
 * séquence noyau validée (le chargement au boot provoque un bootloop → jamais
 * d'autoload, uniquement sur action explicite), puis les actions passives
 * (moniteur, scan, test d'injection, Kismet, capture) se débloquent. La seule
 * action offensive — **deauth ciblé** — passe par [ConfirmSheet], exige la MAC
 * du client, borne le compteur et n'émet en broadcast que sur saisie explicite
 * de ff:ff:ff:ff:ff:ff (incident auto-DoS du 18/11).
 *
 * Modèle d'exécution (cf. RootRunner) :
 *  - noyau/Android (insmod, rmmod, chcon, sysfs, ip…) → [RootRunner.runAndroidRoot]
 *  - aircrack/kismet → chroot Kali via [RootRunner.runStreaming]
 * Chaque action est journalisée dans le fichier suivi par l'agent Wazuh
 * ([RootRunner.logRun]) : traçabilité SOC de l'exécution temps réel.
 */
class AlfaPanel(private val act: AppCompatActivity, root: View, private val ui: CoroutineScope) {

    private val n = Neon(act)
    private val ansi = Regex("\\u001B\\[[0-9;]*[a-zA-Z]")
    private val macRe = Regex("^([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}$")
    private val broadcast = "ff:ff:ff:ff:ff:ff"
    private val sysFmt = SimpleDateFormat("MMM dd HH:mm:ss", Locale.US)
    private val hhmm = SimpleDateFormat("HH:mm:ss", Locale.FRANCE)

    // Emplacements figés sur le téléphone (cf. nethunter-pixel7a-alfa-rt3070-modules).
    private val mod = "/data/adb/modules/nethunter_rt3070"

    // --- Statut réel relu à chaque action (jamais un simple drapeau applicatif) ---
    private data class St(
        val dongle: Boolean = false, val hub: Boolean = false, val modules: Boolean = false,
        val wlan1: Boolean = false, val monitor: Boolean = false, val state: String = "",
        val mac: String? = null, val fw: Boolean = false,
    ) {
        /** « Armé » = dérivé du réel : modules chargés ET interface présente. */
        val armed get() = modules && (wlan1 || monitor)
        /** Déverrouillage des actions : armé ET dongle branché. */
        val unlocked get() = armed && dongle
    }

    private var st = St()
    private var busy = false

    // Vues construites en code et mises à jour par render()/refresh().
    private val content: LinearLayout = root.findViewById(R.id.alfaContent)
    private val console: TextView = root.findViewById(R.id.alfaConsole)
    private val consoleScroll: ScrollView = root.findViewById(R.id.alfaConsoleScroll)

    private lateinit var chipRow: LinearLayout
    private lateinit var macLine: TextView
    private lateinit var toggleBtn: TextView
    private lateinit var toggleState: TextView
    private lateinit var actionsCard: LinearLayout
    private lateinit var lockHint: TextView
    private lateinit var monitorBtn: TextView
    private lateinit var channelInput: EditText
    private val lockables = mutableListOf<TextView>()
    private var firstShown = false

    init {
        root.findViewById<View>(R.id.alfaClear).setOnClickListener { console.text = "" }
        buildUi()
        appendConsole("Alfa RT3070 — carte Wi-Fi USB locale. Périmètre : TON réseau.")
        appendConsole("Jamais d'autoload au boot (bootloop) : charge via « Armer » uniquement.")
    }

    /** Premier statut automatique à l'affichage de l'écran, puis à chaque retour. */
    fun onShown() {
        refresh()
        if (!firstShown) firstShown = true
    }

    private fun dp(v: Int) = (v * act.resources.displayMetrics.density).toInt()
    private fun color(id: Int) = ContextCompat.getColor(act, id)
    private fun toast(s: String) = Toast.makeText(act, s, Toast.LENGTH_SHORT).show()

    private fun appendConsole(s: String) {
        console.append(ansi.replace(s, "") + "\n")
        consoleScroll.post { consoleScroll.fullScroll(ScrollView.FOCUS_DOWN) }
    }

    // ------------------------------------------------------------- construction UI

    private fun buildUi() {
        content.removeAllViews()

        // --- Carte STATUT ---
        content.addView(n.section("Statut", "RT3070 · 148f:3070", n.muted))
        val statusCard = n.card()
        val head = n.row().apply {
            addView(n.kicker("MATÉRIEL LOCAL").apply { layoutParams = n.weight() })
            addView(n.button("↻", Neon.Kind.GHOST) { refresh() }.apply {
                minHeight = dp(32); setPadding(dp(14), dp(4), dp(14), dp(4))
            })
        }
        statusCard.addView(head)
        // Rangée de chips défilable horizontalement : jamais tronquée sur écran étroit.
        chipRow = LinearLayout(act).apply { orientation = LinearLayout.HORIZONTAL }
        statusCard.addView(HorizontalScrollView(act).apply {
            isHorizontalScrollBarEnabled = false
            layoutParams = n.lp().apply { topMargin = dp(8) }
            addView(chipRow)
        })
        macLine = n.monoText("MAC —", 11f, n.muted).apply {
            layoutParams = n.lp().apply { topMargin = dp(8) }
        }
        statusCard.addView(macLine)
        content.addView(statusCard)

        // --- Bascule principale (1 clic) ---
        content.addView(n.section("Mise en œuvre"))
        val armCard = n.card()
        toggleState = n.monoText("NON ARMÉ", 11f, n.muted, bold = true)
        armCard.addView(n.row().apply {
            addView(n.kicker("CARTE").apply { layoutParams = n.weight() })
            addView(toggleState)
        })
        toggleBtn = n.button("ARMER L'ALFA", Neon.Kind.PRIMARY) { onToggle() }.apply {
            layoutParams = n.lp().apply { topMargin = dp(10) }
            minHeight = dp(52); textSize = 15f
        }
        armCard.addView(toggleBtn)
        armCard.addView(n.monoText(
            "Armer = charge les modules RT3070 + firmware (séquence validée, idempotente). " +
            "Désarmer avant de débrancher le dongle.", 10.5f, n.faint).apply {
            layoutParams = n.lp().apply { topMargin = dp(8) }
        })
        content.addView(armCard)

        // --- Actions (verrouillées tant que non armé) ---
        content.addView(n.section("Opérations", "AIRCRACK · KISMET", n.muted))
        actionsCard = LinearLayout(act).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundResource(R.drawable.card_locked)
            setPadding(dp(12), dp(12), dp(12), dp(12))
            layoutParams = n.lp().apply { bottomMargin = dp(8) }
        }
        lockHint = n.monoText("Verrouillé — arme l'Alfa (dongle branché) pour débloquer.", 11f, n.warn).apply {
            layoutParams = n.lp().apply { bottomMargin = dp(10) }
        }
        actionsCard.addView(lockHint)

        // Interface : moniteur + canal
        actionsCard.addView(n.kicker("INTERFACE"))
        monitorBtn = action("Mode moniteur ON/OFF") { onMonitor() }
        actionsCard.addView(monitorBtn)
        val chRow = n.row().apply { layoutParams = n.lp().apply { topMargin = dp(8) } }
        channelInput = EditText(act).apply {
            hint = "canal"; setText("1"); setSingleLine(); textSize = 13f
            typeface = n.mono
            setBackgroundResource(R.drawable.bg_input)
            setTextColor(n.text); setHintTextColor(n.faint)
            inputType = android.text.InputType.TYPE_CLASS_NUMBER
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            layoutParams = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f)
                .apply { marginEnd = dp(8) }
        }
        chRow.addView(channelInput)
        chRow.addView(action("Fixer le canal", wide = false) { onChannel() })
        actionsCard.addView(chRow)

        actionsCard.addView(n.divider())

        // Reconnaissance passive
        actionsCard.addView(n.kicker("RECONNAISSANCE"))
        actionsCard.addView(action("Scan 15 s (APs + clients)") { onScan(15) })
        actionsCard.addView(action("Test d'injection (X/30)") { onInjectionTest() })
        actionsCard.addView(action("Kismet start/stop") { onKismet() })
        actionsCard.addView(action("Capture PCAP ciblée") { onPcap() })

        actionsCard.addView(n.divider())

        // Offensif encadré
        actionsCard.addView(n.kicker("OFFENSIF · ENCADRÉ", n.crit))
        actionsCard.addView(action("Deauth ciblé…", danger = true) { onDeauth() })

        actionsCard.addView(n.divider())

        // Démo
        actionsCard.addView(n.kicker("DÉMO"))
        actionsCard.addView(action("Macro : Armer → Moniteur → Scan 10 s") { onDemo() })

        content.addView(actionsCard)
    }

    /** Bouton d'action plein largeur, enregistré comme verrouillable. */
    private fun action(label: String, wide: Boolean = true, danger: Boolean = false, onClick: () -> Unit): TextView {
        val b = n.button(label, if (danger) Neon.Kind.DANGER else Neon.Kind.SECONDARY, onClick)
        if (wide) b.layoutParams = n.lp().apply { topMargin = dp(8) }
        else { b.minHeight = dp(44); b.setPadding(dp(16), dp(8), dp(16), dp(8)) }
        if (danger) b.minHeight = dp(48)
        lockables += b
        return b
    }

    // ------------------------------------------------------------- statut

    private fun refresh() {
        ui.launch {
            val (_, out) = RootRunner.runAndroidRoot(STATUS)
            st = parseStatus(out)
            renderStatus()
        }
    }

    private val STATUS = """
        echo "DONGLE=${'$'}(for d in /sys/bus/usb/devices/*/idVendor; do v=${'$'}(cat ${'$'}d 2>/dev/null); p=${'$'}(cat ${'$'}{d%idVendor}idProduct 2>/dev/null); if [ "${'$'}v" = "148f" ] && [ "${'$'}p" = "3070" ]; then echo 1; break; fi; done)"
        echo "HUB=${'$'}(for d in /sys/bus/usb/devices/*/idVendor; do v=${'$'}(cat ${'$'}d 2>/dev/null); p=${'$'}(cat ${'$'}{d%idVendor}idProduct 2>/dev/null); if [ "${'$'}v" = "1a40" ] && [ "${'$'}p" = "0101" ]; then echo 1; break; fi; done)"
        echo "MODS=${'$'}(grep -q '^rt2800usb ' /proc/modules && echo 1 || echo 0)"
        echo "WLAN1=${'$'}([ -e /sys/class/net/wlan1 ] && echo 1 || echo 0)"
        echo "MON=${'$'}([ -e /sys/class/net/wlan1mon ] && echo 1 || echo 0)"
        echo "STATE=${'$'}(cat /sys/class/net/wlan1/operstate 2>/dev/null)"
        echo "MAC=${'$'}(cat /sys/class/net/wlan1/address 2>/dev/null || cat /sys/class/net/wlan1mon/address 2>/dev/null)"
        echo "FW=${'$'}([ -f $mod/firmware/rt2870.bin ] && echo 1 || echo 0)"
    """.trimIndent()

    private fun parseStatus(out: String): St {
        val m = HashMap<String, String>()
        for (line in out.lines()) {
            val i = line.indexOf('=')
            if (i > 0) m[line.substring(0, i).trim()] = line.substring(i + 1).trim()
        }
        fun b(k: String) = m[k] == "1"
        return St(
            dongle = b("DONGLE"), hub = b("HUB"), modules = b("MODS"),
            wlan1 = b("WLAN1"), monitor = b("MON"), state = m["STATE"].orEmpty(),
            mac = m["MAC"]?.takeIf { it.isNotBlank() }, fw = b("FW"),
        )
    }

    private fun renderStatus() {
        chipRow.removeAllViews()
        chipRow.addView(chip("DONGLE", if (st.dongle) n.ok else n.crit))
        chipRow.addView(chip("MODULES", if (st.modules) n.ok else n.muted))
        val ifaceColor = when {
            st.monitor -> n.accent
            st.wlan1 && st.state == "up" -> n.ok
            st.wlan1 -> n.warn
            else -> n.crit
        }
        val ifaceLabel = when {
            st.monitor -> "MONITEUR"
            st.wlan1 && st.state == "up" -> "WLAN1 UP"
            st.wlan1 -> "WLAN1 DOWN"
            else -> "WLAN1 —"
        }
        chipRow.addView(chip(ifaceLabel, ifaceColor))
        chipRow.addView(chip("FW", if (st.fw) n.ok else n.crit))

        macLine.text = buildString {
            append(st.mac?.let { "MAC $it" } ?: "MAC —")
            if (st.hub) append("   ·   HUB OTG 1a40:0101")
        }

        // Bascule dérivée du réel.
        if (st.armed) {
            toggleState.text = "ARMÉ"; toggleState.setTextColor(n.accentSoft)
            toggleBtn.text = "DÉSARMER L'ALFA"
            toggleBtn.setBackgroundResource(R.drawable.btn_danger); toggleBtn.setTextColor(color(android.R.color.white))
        } else {
            toggleState.text = "NON ARMÉ"; toggleState.setTextColor(n.muted)
            toggleBtn.text = "ARMER L'ALFA"
            toggleBtn.setBackgroundResource(R.drawable.btn_accent); toggleBtn.setTextColor(color(R.color.bg))
        }

        // Verrou des actions.
        val on = st.unlocked
        lockHint.visibility = if (on) View.GONE else View.VISIBLE
        actionsCard.setBackgroundResource(if (on) R.drawable.card else R.drawable.card_locked)
        for (b in lockables) {
            b.isEnabled = on && !busy
            b.alpha = if (on) 1f else 0.4f
        }
        channelInput.isEnabled = on
        monitorBtn.text = if (st.monitor) "Mode moniteur : OFF" else "Mode moniteur : ON"
    }

    private fun chip(s: String, col: Int) = n.chip(s, col, withDot = true).apply {
        (layoutParams as LinearLayout.LayoutParams).apply { marginEnd = dp(6); bottomMargin = dp(6) }
    }

    // ------------------------------------------------------------- exécution

    /** Journalise dans le fichier suivi par Wazuh (traçabilité SOC temps réel). */
    private fun soc(cmd: String, result: String = "") {
        val line = "${sysFmt.format(Date())} localhost purpleops-alfa: cmd=$cmd" +
            if (result.isNotBlank()) " result=${result.take(120)}" else ""
        ui.launch { RootRunner.logRun(line) }
    }

    /** Commande côté Android root (noyau/sysfs). Retourne la sortie, l'affiche. */
    private suspend fun androidRoot(cmd: String, echo: Boolean = true): String {
        val (_, out) = RootRunner.runAndroidRoot(cmd)
        if (echo) out.lines().filter { it.isNotBlank() }.forEach { appendConsole(it) }
        return out
    }

    /** Commande DANS le chroot Kali ; streame vers la console et renvoie le texte. */
    private suspend fun chroot(inner: String): String {
        val sb = StringBuilder()
        RootRunner.runStreaming(inner) { l -> sb.appendLine(l); ui.launch { appendConsole(l) } }
        return sb.toString()
    }

    private inline fun guard(block: () -> Unit) {
        if (busy) { toast("Une action est en cours…"); return }
        block()
    }

    private fun needMonitor(): Boolean {
        if (!st.monitor) { toast("Active d'abord le mode moniteur"); return false }
        return true
    }

    // ------------------------------------------------------------- bascule

    private fun onToggle() = guard {
        if (!st.armed && !st.dongle) { toast("Branche le dongle OTG"); return@guard }
        if (st.armed) disarm() else arm()
    }

    private fun arm() {
        busy = true; renderStatus()
        appendConsole(""); appendConsole("[${hhmm.format(Date())}] Armement de l'Alfa…")
        soc("alfa-arm")
        ui.launch {
            androidRoot(ARM)
            busy = false
            refresh()
            appendConsole("Armé. Modules RT3070 + firmware chargés (runtime, pas au boot).")
        }
    }

    private fun disarm() {
        busy = true; renderStatus()
        appendConsole(""); appendConsole("[${hhmm.format(Date())}] Désarmement (interface d'abord)…")
        soc("alfa-disarm")
        ui.launch {
            androidRoot(DISARM)
            busy = false
            refresh()
            appendConsole("Carte désarmée : tu peux débrancher le dongle OTG en sécurité.")
        }
    }

    // Séquence VALIDÉE (idempotente). Jamais écrite dans service.d / post-fs-data :
    // aucun autoload au boot (évite le bootloop stack Wi-Fi ↔ OTG mode-hôte).
    private val ARM = """
        MOD=$mod
        FW=/data/local/firmware
        mkdir -p ${'$'}FW
        cp -f ${'$'}MOD/firmware/rt2870.bin ${'$'}FW/rt2870.bin
        chcon u:object_r:vendor_fw_file:s0 ${'$'}FW/rt2870.bin 2>/dev/null
        echo ${'$'}FW > /sys/module/firmware_class/parameters/path
        for m in rt2x00lib rt2x00usb rt2800lib rt2800usb; do
          grep -q "^${'$'}m " /proc/modules || insmod ${'$'}MOD/modules/${'$'}m.ko
        done
        echo "[arm] modules: ${'$'}(lsmod 2>/dev/null | grep -c rt2800) rt2800* chargés"
    """.trimIndent()

    // Déchargement : ordre inverse, interface stoppée avant rmmod (sinon « device busy »).
    private val DISARM = """
        pkill airodump-ng 2>/dev/null; pkill kismet 2>/dev/null; pkill aireplay-ng 2>/dev/null
        iw dev wlan1mon del 2>/dev/null
        ip link set wlan1 down 2>/dev/null
        for m in rt2800usb rt2800lib rt2x00usb rt2x00lib; do
          grep -q "^${'$'}m " /proc/modules && rmmod ${'$'}m
        done
        echo "[disarm] reste: ${'$'}(lsmod 2>/dev/null | grep -c rt2800) rt2800*"
    """.trimIndent()

    // ------------------------------------------------------------- actions passives

    private fun onMonitor() = guard {
        if (!st.unlocked) return@guard
        busy = true; renderStatus()
        ui.launch {
            if (st.monitor) {
                appendConsole("→ airmon-ng stop wlan1mon")
                soc("airmon-ng stop wlan1mon")
                chroot("airmon-ng stop wlan1mon; iw dev wlan1mon del 2>/dev/null; ip link set wlan1 down")
            } else {
                appendConsole("→ airmon-ng start wlan1")
                soc("airmon-ng start wlan1")
                val out = chroot("airmon-ng start wlan1")
                if (!out.contains("wlan1mon") && !out.contains("monitor mode")) {
                    appendConsole("airmon-ng KO → repli iw/ip")
                    chroot("iw dev wlan1 set type monitor && ip link set wlan1 up")
                }
            }
            busy = false; refresh()
        }
    }

    private fun onChannel() = guard {
        if (!st.unlocked || !needMonitor()) return@guard
        val ch = channelInput.text.toString().trim().toIntOrNull()
        if (ch == null || ch !in 1..196) { toast("Canal invalide (1–196)"); return@guard }
        busy = true; renderStatus()
        ui.launch {
            appendConsole("→ canal $ch")
            soc("iw dev wlan1mon set channel $ch")
            chroot("iw dev wlan1mon set channel $ch && echo canal $ch OK")
            busy = false; refresh()
        }
    }

    private fun onScan(secs: Int) = guard {
        if (!st.unlocked || !needMonitor()) return@guard
        busy = true; renderStatus()
        ui.launch {
            appendConsole(""); appendConsole("→ scan $secs s…")
            soc("airodump-ng wlan1mon (${secs}s)")
            chroot("rm -f /tmp/alfa-scan-*; timeout $secs airodump-ng wlan1mon -w /tmp/alfa-scan --output-format csv --write-interval 1 >/dev/null 2>&1; echo SCAN_DONE")
            val csv = RootRunner.readFileAsRoot("/data/local/kali/tmp/alfa-scan-01.csv")
            renderScan(csv)
            busy = false; refresh()
        }
    }

    private fun renderScan(csv: String?) {
        if (csv.isNullOrBlank()) { appendConsole("[scan] aucun résultat (CSV vide)"); return }
        val parts = csv.split("Station MAC")
        val aps = parts[0].lines().mapNotNull { line ->
            val f = line.split(",").map { it.trim() }
            if (f.size < 14 || !macRe.matches(f[0])) null
            else Triple(f[0], f[13].ifBlank { "<caché>" }, "${f[3].ifBlank { "?" }}/${f[8].ifBlank { "?" }}")
        }.sortedByDescending { it.third.substringAfter("/").toIntOrNull() ?: -100 }
        appendConsole("[scan] ${aps.size} AP(s) :")
        for ((bssid, essid, chPwr) in aps.take(12)) appendConsole("  $bssid  ch/pwr $chPwr  $essid")
        if (parts.size > 1) {
            val clients = parts[1].lines().count { macRe.matches(it.split(",").firstOrNull()?.trim().orEmpty()) }
            appendConsole("[scan] $clients client(s) vus")
        }
    }

    private fun onInjectionTest() = guard {
        if (!st.unlocked || !needMonitor()) return@guard
        busy = true; renderStatus()
        ui.launch {
            appendConsole(""); appendConsole("→ aireplay-ng --test wlan1mon")
            soc("aireplay-ng --test wlan1mon")
            val out = chroot("aireplay-ng --test wlan1mon")
            val ratio = Regex("(\\d+)/30").find(out)?.value
            val ok = out.contains("Injection is working", true)
            appendConsole(when {
                ok -> "[injection] OK ${ratio ?: ""}"
                ratio != null -> "[injection] partielle $ratio"
                else -> "[injection] KO (pas de réponse)"
            })
            soc("aireplay-ng --test", if (ok) "ok ${ratio.orEmpty()}" else "ko")
            busy = false; refresh()
        }
    }

    private var kismetOn = false
    private fun onKismet() = guard {
        if (!st.unlocked || !needMonitor()) return@guard
        busy = true; renderStatus()
        ui.launch {
            if (kismetOn) {
                appendConsole("→ arrêt Kismet")
                soc("pkill kismet")
                chroot("pkill kismet; echo KISMET_STOPPED")
                kismetOn = false
            } else {
                appendConsole("→ Kismet (arrière-plan) sur wlan1mon")
                soc("kismet -c wlan1mon")
                chroot("kismet --daemonize -c wlan1mon >/tmp/kismet.log 2>&1; echo KISMET_STARTED")
                kismetOn = true
                appendConsole("[kismet] UI : http://127.0.0.1:2501 (navigateur du tel / tailnet)")
            }
            busy = false; refresh()
        }
    }

    private var pcapOn = false
    private fun onPcap() = guard {
        if (!st.unlocked || !needMonitor()) return@guard
        if (pcapOn) {
            busy = true; renderStatus()
            ui.launch {
                appendConsole("→ arrêt capture PCAP")
                soc("stop airodump capture")
                chroot("pkill airodump-ng; echo PCAP_STOPPED")
                pcapOn = false
                appendConsole("[pcap] fichiers dans /sdcard (alfa-*.cap) — récupère-les via gestionnaire de fichiers / adb pull")
                busy = false; refresh()
            }
            return@guard
        }
        ConfirmSheet(act, "Capture PCAP ciblée", "Démarrer la capture")
            .kicker("RECONNAISSANCE · CARTE ALFA LOCALE")
            .tone(ConfirmSheet.Tone.ACCENT)
            .field("bssid", "BSSID de l'AP ciblé", "AA:BB:CC:DD:EE:FF")
            .field("ch", "Canal", "1", numeric = true, initial = channelInput.text.toString())
            .show { v ->
                val bssid = v["bssid"].orEmpty()
                val ch = v["ch"]?.toIntOrNull()
                if (!macRe.matches(bssid)) { toast("BSSID invalide"); return@show }
                if (ch == null || ch !in 1..196) { toast("Canal invalide"); return@show }
                busy = true; renderStatus()
                val ts = SimpleDateFormat("yyyyMMdd-HHmmss", Locale.US).format(Date())
                ui.launch {
                    appendConsole(""); appendConsole("→ capture PCAP $bssid ch $ch → /sdcard/alfa-$ts")
                    soc("airodump-ng --bssid $bssid -c $ch -w /sdcard/alfa-$ts")
                    chroot("nohup airodump-ng --bssid $bssid -c $ch -w /sdcard/alfa-$ts wlan1mon >/tmp/alfa-pcap.log 2>&1 &")
                    pcapOn = true
                    appendConsole("[pcap] en cours — re-tape « Capture PCAP ciblée » pour arrêter")
                    busy = false; refresh()
                }
            }
    }

    // ------------------------------------------------------------- offensif (encadré)

    private fun onDeauth() = guard {
        if (!st.unlocked || !needMonitor()) return@guard
        ConfirmSheet(act, "Deauth ciblé", "Lancer")
            .kicker("ACTION OFFENSIVE · CARTE ALFA LOCALE")
            .tone(ConfirmSheet.Tone.DANGER)
            .field("bssid", "BSSID de l'AP", "AA:BB:CC:DD:EE:FF")
            .field("client", "MAC du client ciblé (obligatoire)", "aa:bb:cc:dd:ee:ff")
            .field("count", "Nombre de trames (défaut 5, max 50)", "5", numeric = true, initial = "5")
            .warn("Deauth ciblé par défaut. ff:ff:ff:ff:ff:ff (broadcast, à saisir explicitement) " +
                  "éjecte aussi le capteur WIDS et tout client du canal — incident auto-DoS du 18/11. " +
                  "Chaque trame est tracée dans Wazuh.")
            .show { v ->
                val bssid = v["bssid"].orEmpty()
                val client = v["client"].orEmpty().lowercase(Locale.ROOT)
                if (!macRe.matches(bssid)) { toast("BSSID invalide"); return@show }
                if (client.isBlank()) { toast("MAC du client obligatoire (ou ff:ff:ff:ff:ff:ff pour un broadcast explicite)"); return@show }
                if (!macRe.matches(client)) { toast("MAC client invalide"); return@show }
                val count = (v["count"]?.toIntOrNull() ?: 5).coerceIn(1, 50)
                if (client == broadcast) appendConsole("⚠ BROADCAST explicite — collatéral à l'échelle du canal (incident 18/11)")
                val cmd = "aireplay-ng --deauth $count -a $bssid -c $client wlan1mon"
                busy = true; renderStatus()
                ui.launch {
                    appendConsole(""); appendConsole("→ $cmd")
                    soc(cmd)
                    chroot(cmd)
                    soc("deauth terminé", "a=$bssid c=$client n=$count")
                    busy = false; refresh()
                }
            }
    }

    // ------------------------------------------------------------- macro démo

    private fun onDemo() = guard {
        if (!st.dongle) { toast("Branche le dongle OTG"); return@guard }
        busy = true; renderStatus()
        appendConsole(""); appendConsole("=== MACRO DÉMO ===")
        soc("alfa-demo-macro")
        ui.launch {
            if (!st.armed) { appendConsole("[1/3] Armement…"); androidRoot(ARM); refresh() }
            else appendConsole("[1/3] Déjà armé")
            appendConsole("[2/3] Mode moniteur…")
            val mon = chroot("airmon-ng start wlan1")
            if (!mon.contains("wlan1mon") && !mon.contains("monitor mode"))
                chroot("iw dev wlan1 set type monitor && ip link set wlan1 up")
            appendConsole("[3/3] Scan 10 s…")
            chroot("rm -f /tmp/alfa-scan-*; timeout 10 airodump-ng wlan1mon -w /tmp/alfa-scan --output-format csv >/dev/null 2>&1; echo SCAN_DONE")
            renderScan(RootRunner.readFileAsRoot("/data/local/kali/tmp/alfa-scan-01.csv"))
            appendConsole("=== FIN MACRO ===")
            busy = false; refresh()
        }
    }
}
