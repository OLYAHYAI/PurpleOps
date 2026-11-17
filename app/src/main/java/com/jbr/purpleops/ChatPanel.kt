package com.jbr.purpleops

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Chat SOC : le salon #soc:maison d'Element X, dans PurpleOps.
 *
 * - `@agent` (pastille ou saisie) => `m.mentions` : le pont crée un ticket
 *   Paperclip assigné à l'agent, qui répond dans le fil.
 * - Toucher un message = y répondre (répondre à un agent le sollicite aussi).
 * - Synchronisation UNIQUEMENT quand l'écran est affiché (pas de service en
 *   arrière-plan) ; les notifications restent à Element X.
 * - Les messages reçus sont du TEXTE : aucun lien n'est rendu cliquable,
 *   aucun HTML n'est interprété (contenu des agents = données, pas ordres).
 */
class ChatPanel(
    private val act: AppCompatActivity,
    root: View,
    private val ui: CoroutineScope,
    private val onBack: () -> Unit,
) {
    private val ctx: Context get() = act.applicationContext

    private val status: TextView = root.findViewById(R.id.chatStatus)
    private val loginView: View = root.findViewById(R.id.chatLogin)
    private val roomView: View = root.findViewById(R.id.chatRoom)
    private val hsIn: EditText = root.findViewById(R.id.chatHs)
    private val userIn: EditText = root.findViewById(R.id.chatUser)
    private val passIn: EditText = root.findViewById(R.id.chatPass)
    private val loginStatus: TextView = root.findViewById(R.id.chatLoginStatus)
    private val scroll: ScrollView = root.findViewById(R.id.chatScroll)
    private val list: LinearLayout = root.findViewById(R.id.chatList)
    private val agentsRow: LinearLayout = root.findViewById(R.id.chatAgents)
    private val replyBar: View = root.findViewById(R.id.chatReplyBar)
    private val replyText: TextView = root.findViewById(R.id.chatReplyText)
    private val input: EditText = root.findViewById(R.id.chatInput)
    private val sendBtn: TextView = root.findViewById(R.id.chatSend)

    private val cText = ContextCompat.getColor(act, R.color.text)
    private val cMuted = ContextCompat.getColor(act, R.color.muted)
    private val cNeon = ContextCompat.getColor(act, R.color.neon)
    private val cCyan = ContextCompat.getColor(act, R.color.cyan)
    private val cSurface = ContextCompat.getColor(act, R.color.surface)
    private val cSurface2 = ContextCompat.getColor(act, R.color.surface2)
    private val cOk = ContextCompat.getColor(act, R.color.ok)
    private val cWarn = ContextCompat.getColor(act, R.color.warn)
    private val cCrit = ContextCompat.getColor(act, R.color.danger)

    private var session: MatrixClient.Session? = null
    private var members: Map<String, String> = emptyMap()   // user_id -> nom affiché
    private val messages = ArrayList<MatrixClient.Msg>()
    private val seen = HashSet<String>()
    private var since: String? = null
    private var replyTo: MatrixClient.Msg? = null
    private var job: Job? = null
    private var visible = false
    private var lastSend = 0L
    private val hhmm = SimpleDateFormat("HH:mm", Locale.FRANCE)
    private val dayFmt = SimpleDateFormat("EEEE d MMMM", Locale.FRANCE)

    private fun dp(v: Int) = (v * act.resources.displayMetrics.density).toInt()

    init {
        root.findViewById<View>(R.id.chatBack).setOnClickListener { onBack() }
        root.findViewById<View>(R.id.chatMenu).setOnClickListener { menu() }
        root.findViewById<View>(R.id.chatConnect).setOnClickListener { login() }
        root.findViewById<View>(R.id.chatReplyCancel).setOnClickListener { setReply(null) }
        sendBtn.setOnClickListener { send() }
        hsIn.setText(MatrixClient.lastHs(ctx))
        userIn.setText(MatrixClient.lastUser(ctx))
        session = try { MatrixClient.session(ctx) } catch (e: Exception) { null }
        showMode()
    }

    // ------------------------------------------------------------ cycle de vie

    fun setVisible(v: Boolean) {
        visible = v
        if (v) start() else stop()
    }

    /** Ouvre le chat avec un message prérempli (ex. alerte -> « @jbr analyse … »). */
    fun compose(text: String) {
        input.setText(text)
        input.setSelection(input.text.length)
        input.requestFocus()
    }

    private fun start() {
        if (session == null || job?.isActive == true) return
        job = ui.launch {
            var backoff = 3_000L
            while (isActive && visible) {
                val s = session ?: break
                try {
                    if (members.isEmpty()) { members = MatrixClient.members(s); renderAgents() }
                    val r = MatrixClient.sync(ctx, s, since, 25_000)
                    if (!isActive || !visible || session !== s) break
                    val first = since == null
                    since = r.nextBatch
                    if (r.membersChanged && !first) { members = MatrixClient.members(s); renderAgents() }
                    if (r.messages.isNotEmpty()) add(r.messages)
                    setStatus("en ligne", cOk)
                    backoff = 3_000L
                } catch (e: MatrixClient.MxError) {
                    if (e.status == 401 || e.errcode == "M_UNKNOWN_TOKEN") {
                        MatrixClient.forget(ctx); session = null; since = null
                        loginStatus.text = "Session révoquée ou expirée : reconnecte-toi."
                        showMode(); break
                    }
                    setStatus("erreur ${e.errcode}", cCrit)
                    delay(backoff); backoff = minOf(backoff * 2, 60_000L)
                } catch (e: Exception) {
                    setStatus("hors ligne", cWarn)
                    delay(backoff); backoff = minOf(backoff * 2, 60_000L)
                }
            }
        }
    }

    private fun stop() { job?.cancel(); job = null }

    private fun showMode() {
        val s = session
        loginView.visibility = if (s == null) View.VISIBLE else View.GONE
        roomView.visibility = if (s == null) View.GONE else View.VISIBLE
        if (s == null) setStatus("déconnecté", cMuted) else { setStatus("connexion…", cMuted); if (visible) start() }
    }

    private fun setStatus(t: String, c: Int) { status.text = t; status.setTextColor(c) }

    // ------------------------------------------------------------ session

    private fun login() {
        val pass = passIn.text.toString()
        if (pass.isEmpty()) { toast("Mot de passe requis"); return }
        loginStatus.text = "Connexion…"
        ui.launch {
            try {
                val s = MatrixClient.login(ctx, hsIn.text.toString(), userIn.text.toString(), pass)
                passIn.setText("")
                session = s; since = null; members = emptyMap(); messages.clear(); seen.clear(); list.removeAllViews()
                loginStatus.text = ""
                toast("Connecté : ${s.userId} (appareil ${s.deviceId})")
                showMode()
            } catch (e: MatrixClient.MxError) {
                loginStatus.text = when (e.errcode) {
                    "M_FORBIDDEN" -> "Identifiants refusés."
                    "M_LIMIT_EXCEEDED" -> "Trop de tentatives : patiente un peu."
                    "M_NOT_FOUND" -> "Salon #soc:maison introuvable."
                    else -> "Échec : ${e.errcode} — ${e.message}"
                }
            } catch (e: Exception) {
                loginStatus.text = "Serveur injoignable (Tailscale actif ?) : ${e.message ?: e.javaClass.simpleName}"
            }
        }
    }

    private fun menu() {
        val s = session
        if (s == null) { toast("Non connecté"); return }
        AlertDialog.Builder(act)
            .setTitle("Session Matrix")
            .setMessage("${s.userId}\nappareil ${s.deviceId.ifBlank { "?" }}\nsalon ${s.roomId}\n\nDÉCONNECTER révoque cette session sur le serveur (Element X n'est pas touché).")
            .setPositiveButton("Déconnecter") { _, _ ->
                stop()
                ui.launch {
                    MatrixClient.logout(ctx)
                    session = null; since = null; members = emptyMap(); messages.clear(); seen.clear()
                    list.removeAllViews(); agentsRow.removeAllViews(); setReply(null)
                    showMode()
                }
            }
            .setNeutralButton("Recharger") { _, _ ->
                stop(); since = null; members = emptyMap(); messages.clear(); seen.clear(); list.removeAllViews(); start()
            }
            .setNegativeButton("Fermer", null).show()
    }

    // ------------------------------------------------------------ affichage

    private fun me() = session?.userId.orEmpty()
    private fun local(uid: String) = uid.removePrefix("@").substringBefore(':')
    private fun name(uid: String) = members[uid] ?: local(uid)

    private fun renderAgents() {
        agentsRow.removeAllViews()
        val others = members.keys.filter { it != me() }.sortedBy { if (local(it) == "paperclip") "~" else local(it) }
        for (uid in others) {
            agentsRow.addView(TextView(act).apply {
                text = "@${local(uid)}"
                setTextColor(if (local(uid) == "paperclip") cMuted else cCyan)
                textSize = 12.5f
                setBackgroundResource(R.drawable.btn_neon)
                setPadding(dp(12), dp(7), dp(12), dp(7))
                layoutParams = LinearLayout.LayoutParams(-2, -2).apply { marginEnd = dp(6) }
                setOnClickListener { insertMention(uid) }
            })
        }
    }

    private fun insertMention(uid: String) {
        val tag = "@${local(uid)} "
        val pos = input.selectionStart.coerceAtLeast(0)
        val t = input.text
        val prefix = if (pos > 0 && !t[pos - 1].isWhitespace()) " " else ""
        t.insert(pos, prefix + tag)
        input.requestFocus()
    }

    private fun add(incoming: List<MatrixClient.Msg>) {
        val atBottom = !scroll.canScrollVertically(1)
        var changed = false
        for (m in incoming) if (seen.add(m.eventId)) { messages += m; changed = true }
        if (!changed) return
        messages.sortBy { it.ts }
        while (messages.size > 200) seen.remove(messages.removeAt(0).eventId)
        render()
        if (atBottom || incoming.any { it.sender == me() }) scroll.post { scroll.fullScroll(View.FOCUS_DOWN) }
    }

    private fun render() {
        list.removeAllViews()
        val byId = messages.associateBy { it.eventId }
        var lastDay = ""
        for (m in messages) {
            val day = dayFmt.format(Date(m.ts))
            if (day != lastDay) {
                lastDay = day
                list.addView(TextView(act).apply {
                    text = day; setTextColor(cMuted); textSize = 11f; gravity = Gravity.CENTER
                    setPadding(0, dp(8), 0, dp(6))
                    layoutParams = LinearLayout.LayoutParams(-1, -2)
                })
            }
            list.addView(bubble(m, m.replyTo?.let { byId[it] }))
        }
        if (messages.isEmpty()) list.addView(TextView(act).apply {
            text = "Aucun message récent dans #soc."; setTextColor(cMuted); textSize = 13f; setPadding(dp(4), dp(12), dp(4), dp(12))
        })
    }

    private fun bubble(m: MatrixClient.Msg, parent: MatrixClient.Msg?): View {
        val mine = m.sender == me()
        val forMe = !mine && me() in m.mentions
        val box = LinearLayout(act).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(12), dp(8), dp(12), dp(9))
            background = GradientDrawable().apply {
                cornerRadius = dp(12).toFloat()
                setColor(if (mine) cSurface2 else cSurface)
                setStroke(dp(1), if (forMe) cNeon else Color.parseColor("#241C3A"))
            }
            layoutParams = LinearLayout.LayoutParams(-1, -2).apply {
                bottomMargin = dp(6)
                if (mine) marginStart = dp(36) else marginEnd = dp(36)
            }
            setOnClickListener { setReply(m) }
            setOnLongClickListener { actions(m); true }
        }
        val head = LinearLayout(act).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
        head.addView(TextView(act).apply {
            text = if (mine) "moi" else name(m.sender)
            setTextColor(when { mine -> cNeon; local(m.sender) == "paperclip" -> cMuted; else -> cCyan })
            textSize = 12.5f; setTypeface(null, Typeface.BOLD)
            layoutParams = LinearLayout.LayoutParams(0, -2, 1f)
        })
        head.addView(TextView(act).apply { text = hhmm.format(Date(m.ts)); setTextColor(cMuted); textSize = 10.5f })
        box.addView(head)
        if (m.replyTo != null) box.addView(TextView(act).apply {
            text = if (parent != null) "↪ ${if (parent.sender == me()) "moi" else name(parent.sender)} : ${parent.body.replace('\n', ' ').take(90)}"
            else "↪ réponse à un message plus ancien"
            setTextColor(cMuted); textSize = 11f; maxLines = 2
            setPadding(0, dp(3), 0, dp(1))
        })
        box.addView(TextView(act).apply {
            text = m.body
            setTextColor(if (m.notice) cMuted else cText)
            textSize = if (m.notice) 12.5f else 14f
            if (m.notice) setTypeface(null, Typeface.ITALIC)
            setPadding(0, dp(3), 0, 0)
            // Pas d'autoLink : le contenu des agents est une donnée.
        })
        return box
    }

    private fun actions(m: MatrixClient.Msg) {
        AlertDialog.Builder(act)
            .setItems(arrayOf("Répondre", "Copier le texte", "Copier l'identifiant (event_id)")) { _, i ->
                when (i) {
                    0 -> setReply(m)
                    1 -> copy(m.body)
                    2 -> copy(m.eventId)
                }
            }.show()
    }

    private fun setReply(m: MatrixClient.Msg?) {
        replyTo = m
        replyBar.visibility = if (m == null) View.GONE else View.VISIBLE
        if (m != null) {
            val who = if (m.sender == me()) "moi" else name(m.sender)
            val agent = m.sender != me() && local(m.sender) != "paperclip"
            replyText.text = "En réponse à $who${if (agent) " (sollicite l'agent)" else ""} : ${m.body.replace('\n', ' ').take(120)}"
            input.requestFocus()
        }
    }

    // ------------------------------------------------------------ envoi

    private fun send() {
        val s = session ?: return
        val text = input.text.toString().trim()
        if (text.isEmpty()) return
        val now = System.currentTimeMillis()
        if (now - lastSend < 1_500) { toast("Doucement : 1 message / 1,5 s"); return }
        // Mentions = membres dont « @localpart » apparaît dans le texte.
        val mentions = LinkedHashMap<String, String>()
        for (uid in members.keys) {
            if (uid == s.userId) continue
            if (Regex("@" + Regex.escape(local(uid)) + "\\b", RegexOption.IGNORE_CASE).containsMatchIn(text)) mentions[uid] = name(uid)
        }
        val rt = replyTo
        lastSend = now
        sendBtn.isEnabled = false
        ui.launch {
            try {
                MatrixClient.send(s, text, mentions, rt?.eventId)
                input.setText(""); setReply(null)
                if (mentions.isEmpty() && rt == null) toast("Envoyé (aucun agent mentionné : pas de ticket)")
            } catch (e: MatrixClient.MxError) {
                toast("Refusé : ${e.errcode}")
            } catch (e: Exception) {
                toast("Non envoyé : ${e.message ?: e.javaClass.simpleName}")
            } finally {
                sendBtn.isEnabled = true
            }
        }
    }

    private fun copy(s: String) {
        (act.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("soc", s))
        toast("Copié")
    }

    private fun toast(s: String) = Toast.makeText(act, s, Toast.LENGTH_SHORT).show()
}
