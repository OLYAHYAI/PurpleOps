package com.jbr.purpleops

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * Repli du réveil quand wol-pi est injoignable depuis le téléphone.
 *
 * Le téléphone appelle le webhook n8n `wol-wake` (workflow « Reveiller un noeud
 * Proxmox (wol-pi) »). n8n retente wol-pi, puis, en cas d'échec, envoie le
 * paquet magique par l'API Proxmox d'un nœud allumé (POST /nodes/<n>/wakeonlan,
 * jeton `wake@pve!n8n` gardé DANS n8n : aucun jeton Proxmox sur le téléphone).
 *
 * Authentification : en-tête `X-PurpleOps-Key` (secret chiffré par [SecureStore]).
 * Le webhook est publié par Tailscale Funnel : sans la clé, n8n répond 403.
 *
 * Limites : n8n tourne sur F4BLOX, donc ce repli ne peut pas réveiller F4BLOX,
 * ni rien si F4BLOX est éteint. PBS n'a pas de repli (hors cluster).
 */
object WakeRelay {

    const val DEFAULT_URL = "https://n8n-soar.tail8a4866.ts.net/webhook/wol-wake"
    const val P_URL = "wol_relay_url"
    const val P_KEY = "wol_relay_key"   // secret (SecureStore)
    private const val HEADER = "X-PurpleOps-Key"

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("purpleops", Context.MODE_PRIVATE)
    fun url(ctx: Context): String = prefs(ctx).getString(P_URL, DEFAULT_URL).orEmpty().ifBlank { DEFAULT_URL }
    fun isConfigured(ctx: Context) = SecureStore.get(ctx, P_KEY).isNotEmpty()

    fun setKey(ctx: Context, key: String) {
        val ed = prefs(ctx).edit()
        SecureStore.put(ed, P_KEY, key.trim())
        ed.apply()
    }

    /** Réponse du webhook. [result] : sent | already_up | no_link | error | rejected | ssh_error | fallback_error | relay_error. */
    data class Result(val result: String, val method: String, val wolpiResult: String, val detail: String)

    /** Appelle le webhook. Jamais d'exception : une panne devient `relay_error`. */
    suspend fun wake(ctx: Context, target: String): Result = withContext(Dispatchers.IO) {
        val key = SecureStore.get(ctx, P_KEY)
        if (key.isEmpty()) return@withContext Result("relay_error", "n8n", "", "repli n8n non configuré (Réglages)")
        val u = URL(url(ctx))
        if (u.protocol != "https") return@withContext Result("relay_error", "n8n", "", "HTTPS obligatoire")
        val conn = (u.openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            connectTimeout = 10_000
            readTimeout = 45_000 // n8n retente wol-pi (SSH, ~12 s) puis l'API Proxmox (10 s)
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("Accept", "application/json")
            setRequestProperty(HEADER, key)
        }
        try {
            conn.outputStream.use { it.write(JSONObject().put("target", target).toString().toByteArray(Charsets.UTF_8)) }
            val code = conn.responseCode
            val stream = if (code in 200..299) conn.inputStream else conn.errorStream
            val text = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
            when {
                code == 401 || code == 403 -> Result("relay_error", "n8n", "", "clé du repli refusée par n8n (HTTP $code)")
                code !in 200..299 -> Result("relay_error", "n8n", "", "n8n HTTP $code")
                else -> {
                    val j = JSONObject(text)
                    Result(
                        j.optString("result", "relay_error"),
                        j.optString("method", "n8n"),
                        j.optString("wolpi_result", ""),
                        j.optString("detail", ""),
                    )
                }
            }
        } catch (e: Exception) {
            Result("relay_error", "n8n", "", "n8n injoignable (${e.javaClass.simpleName})")
        } finally {
            conn.disconnect()
        }
    }
}
