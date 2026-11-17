package com.jbr.purpleops

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder

/**
 * Client Matrix minimal pour le salon SOC (#soc:maison, Tuwunel sur le tailnet).
 *
 * Périmètre volontairement réduit : UN salon, texte seul, pas d'E2EE (le salon
 * n'est pas chiffré : les agents n'ont pas d'E2EE). API client-serveur v3 en
 * direct (login, filter, sync, joined_members, send, logout).
 *
 * Le pont `matrix-soc-bridge` (VM 122) ne traite que les messages des humains de
 * confiance (`owners` de room.json : @f4blox, @lineageos — compte du téléphone).
 * L'appli ouvre une SESSION DÉDIÉE de ce compte (appareil « PurpleOps »),
 * révocable seule depuis Element X ou ici (DÉCONNECTER). Le mot de passe n'est
 * jamais stocké ; seul le jeton d'accès l'est, chiffré par SecureStore (Keystore).
 *
 * Certificat : chat.tail8a4866.ts.net présente un certificat Let's Encrypt
 * (tailscale serve), validé par le magasin système. Pas d'HTTP en clair.
 */
object MatrixClient {

    const val DEFAULT_HS = "https://chat.tail8a4866.ts.net"
    const val DEFAULT_ROOM = "#soc:maison"
    const val DEFAULT_USER = "lineageos"
    const val MAX_LEN = 4000

    private const val P_HS = "mx_hs"
    private const val P_USER = "mx_user"
    private const val P_DEVICE = "mx_device"
    private const val P_ROOM = "mx_room"
    private const val P_FILTER = "mx_filter"
    const val P_TOKEN = "mx_token"   // secret (SecureStore)

    class MxError(val status: Int, val errcode: String, msg: String) : Exception(msg)

    data class Session(val hs: String, val userId: String, val deviceId: String, val token: String, val roomId: String)

    data class Msg(
        val eventId: String,
        val sender: String,
        val ts: Long,
        val body: String,
        val notice: Boolean,
        val replyTo: String?,
        val mentions: List<String>,
    )

    data class SyncResult(val nextBatch: String, val messages: List<Msg>, val membersChanged: Boolean)

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("purpleops", Context.MODE_PRIVATE)

    fun session(ctx: Context): Session? {
        val p = prefs(ctx)
        val token = SecureStore.get(ctx, P_TOKEN)
        val hs = p.getString(P_HS, null) ?: return null
        val user = p.getString(P_USER, null) ?: return null
        val room = p.getString(P_ROOM, null) ?: return null
        if (token.isEmpty()) return null
        return Session(hs, user, p.getString(P_DEVICE, "").orEmpty(), token, room)
    }

    fun lastHs(ctx: Context) = prefs(ctx).getString(P_HS, null) ?: DEFAULT_HS
    fun lastUser(ctx: Context) = prefs(ctx).getString(P_USER, null)?.removePrefix("@")?.substringBefore(':') ?: DEFAULT_USER

    // ------------------------------------------------------------ HTTP

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8").replace("+", "%20")

    private fun call(
        hs: String, method: String, path: String, token: String?, body: JSONObject? = null, readTimeoutMs: Int = 15_000,
    ): JSONObject {
        val url = URL(hs.trimEnd('/') + path)
        require(url.protocol == "https") { "HTTPS obligatoire" }
        val c = url.openConnection() as HttpURLConnection
        try {
            c.requestMethod = method
            c.connectTimeout = 8_000
            c.readTimeout = readTimeoutMs
            c.setRequestProperty("Accept", "application/json")
            if (token != null) c.setRequestProperty("Authorization", "Bearer $token")
            if (body != null) {
                c.doOutput = true
                c.setRequestProperty("Content-Type", "application/json")
                c.outputStream.use { it.write(body.toString().toByteArray(Charsets.UTF_8)) }
            }
            val code = c.responseCode
            val stream = if (code in 200..299) c.inputStream else c.errorStream
            val txt = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
            val json = try { JSONObject(txt.ifBlank { "{}" }) } catch (e: Exception) { JSONObject() }
            if (code !in 200..299) {
                throw MxError(code, json.optString("errcode", "HTTP_$code"), json.optString("error", "HTTP $code"))
            }
            return json
        } finally {
            c.disconnect()
        }
    }

    // ------------------------------------------------------------ session

    /** Ouvre une session dédiée « PurpleOps » ; le mot de passe n'est pas conservé. */
    suspend fun login(ctx: Context, hsIn: String, userIn: String, password: String, roomIn: String = DEFAULT_ROOM): Session =
        withContext(Dispatchers.IO) {
            val hs = hsIn.trim().trimEnd('/').ifBlank { DEFAULT_HS }
            val user = userIn.trim().removePrefix("@").substringBefore(':').ifBlank { DEFAULT_USER }
            val res = call(hs, "POST", "/_matrix/client/v3/login", null, JSONObject()
                .put("type", "m.login.password")
                .put("identifier", JSONObject().put("type", "m.id.user").put("user", user))
                .put("password", password)
                .put("initial_device_display_name", "PurpleOps (téléphone)"))
            val token = res.getString("access_token")
            val userId = res.getString("user_id")
            val deviceId = res.optString("device_id")
            val roomId = try { resolveRoom(hs, token, roomIn) } catch (e: Exception) {
                // Le jeton vient d'être créé : ne pas laisser une session orpheline.
                try { call(hs, "POST", "/_matrix/client/v3/logout", token, JSONObject()) } catch (_: Exception) { }
                throw e
            }
            val ed = prefs(ctx).edit()
                .putString(P_HS, hs).putString(P_USER, userId).putString(P_DEVICE, deviceId)
                .putString(P_ROOM, roomId).remove(P_FILTER)
            SecureStore.put(ed, P_TOKEN, token)
            ed.commit()
            Session(hs, userId, deviceId, token, roomId)
        }

    private fun resolveRoom(hs: String, token: String, room: String): String {
        val r = room.trim().ifBlank { DEFAULT_ROOM }
        if (r.startsWith("!")) return r
        return call(hs, "GET", "/_matrix/client/v3/directory/room/${enc(r)}", token).getString("room_id")
    }

    /** Révoque la session côté serveur (best effort) puis efface le jeton local. */
    suspend fun logout(ctx: Context) = withContext(Dispatchers.IO) {
        session(ctx)?.let { s ->
            try { call(s.hs, "POST", "/_matrix/client/v3/logout", s.token, JSONObject()) } catch (_: Exception) { }
        }
        forget(ctx)
    }

    fun forget(ctx: Context) {
        val ed = prefs(ctx).edit().remove(P_DEVICE).remove(P_ROOM).remove(P_FILTER)
        SecureStore.put(ed, P_TOKEN, "")
        ed.commit()
    }

    // ------------------------------------------------------------ lecture

    /** Membres présents du salon : user_id -> nom affiché. */
    suspend fun members(s: Session): Map<String, String> = withContext(Dispatchers.IO) {
        val joined = call(s.hs, "GET", "/_matrix/client/v3/rooms/${enc(s.roomId)}/joined_members", s.token).optJSONObject("joined")
            ?: JSONObject()
        val out = LinkedHashMap<String, String>()
        for (uid in joined.keys()) {
            val dn = joined.optJSONObject(uid)?.optString("display_name").orEmpty()
            out[uid] = dn.ifBlank { uid.removePrefix("@").substringBefore(':') }
        }
        out
    }

    /** Filtre serveur : uniquement le salon SOC, messages + changements de membres. */
    private fun filterId(ctx: Context, s: Session): String {
        prefs(ctx).getString(P_FILTER, null)?.let { return it }
        val notAll = JSONObject().put("not_types", JSONArray().put("*"))
        val f = JSONObject()
            .put("presence", notAll)
            .put("account_data", notAll)
            .put("room", JSONObject()
                .put("rooms", JSONArray().put(s.roomId))
                .put("timeline", JSONObject().put("limit", 40)
                    .put("types", JSONArray().put("m.room.message").put("m.room.member")))
                .put("state", JSONObject().put("types", JSONArray().put("m.room.member")).put("lazy_load_members", true))
                .put("ephemeral", notAll)
                .put("account_data", notAll))
        val id = call(s.hs, "POST", "/_matrix/client/v3/user/${enc(s.userId)}/filter", s.token, f).getString("filter_id")
        prefs(ctx).edit().putString(P_FILTER, id).apply()
        return id
    }

    /**
     * Un tour de /sync. Sans `since` : derniers messages du salon (rattrapage).
     * Avec `since` : attente longue (timeoutMs) de nouveaux évènements.
     */
    suspend fun sync(ctx: Context, s: Session, since: String?, timeoutMs: Int): SyncResult = withContext(Dispatchers.IO) {
        val fid = filterId(ctx, s)
        val q = StringBuilder("/_matrix/client/v3/sync?filter=${enc(fid)}&timeout=${if (since == null) 0 else timeoutMs}")
        if (since != null) q.append("&since=").append(enc(since))
        val res = call(s.hs, "GET", q.toString(), s.token, readTimeoutMs = timeoutMs + 20_000)
        val room = res.optJSONObject("rooms")?.optJSONObject("join")?.optJSONObject(s.roomId)
        val events = room?.optJSONObject("timeline")?.optJSONArray("events") ?: JSONArray()
        val msgs = ArrayList<Msg>()
        var members = false
        for (i in 0 until events.length()) {
            val ev = events.optJSONObject(i) ?: continue
            when (ev.optString("type")) {
                "m.room.member" -> members = true
                "m.room.message" -> parse(ev)?.let { msgs += it }
            }
        }
        if ((room?.optJSONObject("state")?.optJSONArray("events")?.length() ?: 0) > 0) members = true
        SyncResult(res.getString("next_batch"), msgs, members)
    }

    private fun parse(ev: JSONObject): Msg? {
        val c = ev.optJSONObject("content") ?: return null
        val type = c.optString("msgtype")
        if (type.isEmpty()) return null // message supprimé (redaction)
        // Éditions : on garde le message d'origine (comme le pont, qui les ignore).
        if (c.optJSONObject("m.relates_to")?.optString("rel_type") == "m.replace") return null
        val reply = c.optJSONObject("m.relates_to")?.optJSONObject("m.in_reply_to")?.optString("event_id")?.ifBlank { null }
        var body = c.optString("body")
        if (reply != null) body = stripReplyFallback(body)
        if (type != "m.text" && type != "m.notice" && type != "m.emote") body = "[${type.removePrefix("m.")}] $body"
        val mentions = c.optJSONObject("m.mentions")?.optJSONArray("user_ids")?.let { a -> (0 until a.length()).map { a.optString(it) } }
            ?: emptyList()
        return Msg(ev.optString("event_id"), ev.optString("sender"), ev.optLong("origin_server_ts"), body,
            type == "m.notice", reply, mentions)
    }

    /** Retire la citation « > <@x> … » ajoutée en tête des réponses par les clients historiques. */
    fun stripReplyFallback(body: String): String {
        val lines = body.split('\n')
        var i = 0
        while (i < lines.size && lines[i].startsWith(">")) i++
        if (i == 0) return body
        while (i < lines.size && lines[i].isBlank()) i++
        return lines.drop(i).joinToString("\n")
    }

    // ------------------------------------------------------------ envoi

    /**
     * Envoie un texte. Les mentions sont posées dans `m.mentions` (ce que lit le
     * pont) et en liens matrix.to dans `formatted_body` (pastilles Element X).
     * Le texte de l'utilisateur est échappé : aucun HTML libre n'est envoyé.
     */
    suspend fun send(s: Session, text: String, mentions: Map<String, String>, replyTo: String?): String =
        withContext(Dispatchers.IO) {
            val body = text.take(MAX_LEN)
            var html = escapeHtml(body)
            // mentions : user_id -> nom affiché ; le texte contient « @localpart ».
            for ((uid, name) in mentions) {
                val local = uid.removePrefix("@").substringBefore(':')
                html = html.replace(Regex("@" + Regex.escape(local) + "\\b", RegexOption.IGNORE_CASE),
                    Regex.escapeReplacement("<a href=\"https://matrix.to/#/${escapeHtml(uid)}\">${escapeHtml(name)}</a>"))
            }
            html = html.replace("\n", "<br>")
            val content = JSONObject()
                .put("msgtype", "m.text")
                .put("body", body)
                .put("format", "org.matrix.custom.html")
                .put("formatted_body", html)
                .put("m.mentions", JSONObject().put("user_ids", JSONArray(mentions.keys.toList())))
            if (replyTo != null) content.put("m.relates_to", JSONObject().put("m.in_reply_to", JSONObject().put("event_id", replyTo)))
            val txn = "po" + System.currentTimeMillis() + (0..9999).random()
            call(s.hs, "PUT", "/_matrix/client/v3/rooms/${enc(s.roomId)}/send/m.room.message/$txn", s.token, content)
                .optString("event_id")
        }

    private fun escapeHtml(s: String) = s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;")
}
