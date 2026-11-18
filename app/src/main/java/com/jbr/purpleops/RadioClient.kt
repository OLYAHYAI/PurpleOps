package com.jbr.purpleops

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * Plan de contrôle radio du capteur wol-pi, via le webhook n8n (workflow
 * « WIDS radio control », SSH commande imposée `wids-ctl`).
 *
 * L'offensif reste côté serveur : le téléphone n'appelle qu'un endpoint HTTP
 * borné (actions figées), le Pi re-valide tout (whitelist, allowlist de BSSID,
 * détection carte d'injection) et audite chaque action dans Wazuh. L'URL par
 * défaut vise l'instance n8n sur le tailnet (certificat Tailscale valide, pas
 * d'épinglage nécessaire, contrairement à Wazuh auto-signé).
 */
object RadioClient {

    const val DEFAULT_URL = "https://n8n-soar.tail8a4866.ts.net/webhook/wids-ctl"
    const val P_URL = "wids_webhook"
    val ACTIONS = setOf("status", "sniff", "kismet-start", "kismet-stop", "capture-handshake", "deauth-test")

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("purpleops", Context.MODE_PRIVATE)
    fun url(ctx: Context): String = prefs(ctx).getString(P_URL, DEFAULT_URL).orEmpty().ifBlank { DEFAULT_URL }
    fun setUrl(ctx: Context, u: String) { prefs(ctx).edit().putString(P_URL, u.trim()).apply() }

    data class Result(
        val ok: Boolean,
        val action: String,
        val fields: Map<String, String>,
        val raw: String,
        val error: String?
    )

    /** Appelle le webhook ; renvoie l'état parsé ou l'erreur. Jamais d'exception. */
    suspend fun call(ctx: Context, action: String, args: String = ""): Result = withContext(Dispatchers.IO) {
        if (action !in ACTIONS) return@withContext Result(false, action, emptyMap(), "", "action hors liste")
        val body = JSONObject().put("action", action).put("args", args).toString()
        val conn = (URL(url(ctx)).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = 10_000
            readTimeout = 75_000 // sniff/capture peuvent durer côté Pi
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("Accept", "application/json")
        }
        try {
            conn.outputStream.use { it.write(body.toByteArray(Charsets.UTF_8)) }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
            if (code !in 200..299) return@withContext Result(false, action, emptyMap(), text, "HTTP $code")
            val j = JSONObject(text)
            val fields = HashMap<String, String>()
            val fobj = j.optJSONObject("fields")
            if (fobj != null) {
                val it = fobj.keys()
                while (it.hasNext()) { val k = it.next(); fields[k] = fobj.optString(k) }
            }
            Result(j.optBoolean("ok", false), j.optString("action", action), fields, j.optString("raw", text), null)
        } catch (e: Exception) {
            Result(false, action, emptyMap(), "", "${e.javaClass.simpleName}: ${e.message?.take(120)}")
        } finally {
            conn.disconnect()
        }
    }
}
