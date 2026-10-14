package com.jbr.purpleops

import android.content.Context
import android.util.Base64
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager

/** Agent Wazuh (API manager). */
data class WzAgent(val id: String, val name: String, val ip: String, val status: String, val lastKeepAlive: String)

/** Alerte (Indexer). */
data class WzAlert(val desc: String, val level: Int, val agent: String, val ts: String)

/** Synthèse pour le dashboard. */
data class WzSummary(
    val total: Int, val active: Int, val disconnected: Int,
    val alerts24h: Int, val score: Int, val statusLabel: String,
    val agents: List<WzAgent>, val alerts: List<WzAlert>
)

sealed class WzResult {
    data class Ok(val summary: WzSummary) : WzResult()
    data class Err(val message: String) : WzResult()
    object NotConfigured : WzResult()
}

/**
 * Client Wazuh : API manager (port 55000, JWT) + Indexer (port 9200, Basic).
 * Joignable via Tailscale. Les certificats sont self-signed → on fait confiance
 * à tout certificat (acceptable car flux Tailscale homelab uniquement ; ne PAS
 * réutiliser tel quel hors de ce contexte).
 */
object WazuhClient {

    private const val API_PORT = 55000
    private const val IDX_PORT = 9200

    private data class Cfg(val host: String, val apiUser: String, val apiPass: String, val idxUser: String, val idxPass: String)

    private fun cfg(ctx: Context): Cfg? {
        val p = ctx.getSharedPreferences("purpleops", Context.MODE_PRIVATE)
        val host = p.getString("wz_host", "")?.trim().orEmpty()
        val au = p.getString("wz_api_user", "").orEmpty()
        val ap = p.getString("wz_api_pass", "").orEmpty()
        val iu = p.getString("wz_idx_user", "").orEmpty()
        val ip = p.getString("wz_idx_pass", "").orEmpty()
        if (host.isEmpty() || au.isEmpty() || ap.isEmpty()) return null
        return Cfg(host, au, ap, iu, ip)
    }

    // SSLContext qui fait confiance à tout (self-signed Tailscale).
    private val trustAll: SSLContext by lazy {
        val tm = arrayOf<TrustManager>(object : X509TrustManager {
            override fun checkClientTrusted(c: Array<out java.security.cert.X509Certificate>?, a: String?) {}
            override fun checkServerTrusted(c: Array<out java.security.cert.X509Certificate>?, a: String?) {}
            override fun getAcceptedIssuers(): Array<java.security.cert.X509Certificate> = arrayOf()
        })
        SSLContext.getInstance("TLS").apply { init(null, tm, java.security.SecureRandom()) }
    }

    private fun open(urlStr: String): HttpURLConnection {
        val c = URL(urlStr).openConnection() as HttpURLConnection
        if (c is HttpsURLConnection) {
            c.sslSocketFactory = trustAll.socketFactory
            c.setHostnameVerifier { _, _ -> true }
        }
        c.connectTimeout = 8000
        c.readTimeout = 15000
        return c
    }

    private fun basic(user: String, pass: String): String =
        "Basic " + Base64.encodeToString("$user:$pass".toByteArray(), Base64.NO_WRAP)

    suspend fun load(ctx: Context): WzResult = withContext(Dispatchers.IO) {
        val cfg = cfg(ctx) ?: return@withContext WzResult.NotConfigured
        try {
            val token = authenticate(cfg) ?: return@withContext WzResult.Err("Auth API échouée (identifiants/port 55000 ?)")
            val agents = getAgents(cfg, token)
            val alerts = getAlerts(cfg, 30)

            val total = agents.size
            val active = agents.count { it.status.equals("active", true) }
            val disc = total - active
            val high = alerts.count { it.level >= 10 }
            var score = 100 - minOf(40, disc * 8) - minOf(40, high * 5)
            if (score < 0) score = 0
            val label = when {
                score >= 85 -> "Protégé"
                score >= 60 -> "Attention"
                else -> "Alerte"
            }
            WzResult.Ok(WzSummary(total, active, disc, alerts.size, score, label, agents, alerts))
        } catch (e: Exception) {
            WzResult.Err(e.message ?: "erreur réseau")
        }
    }

    private fun authenticate(cfg: Cfg): String? {
        val c = open("https://${cfg.host}:$API_PORT/security/user/authenticate?raw=true")
        c.requestMethod = "POST"
        c.setRequestProperty("Authorization", basic(cfg.apiUser, cfg.apiPass))
        return try {
            if (c.responseCode == 200) c.inputStream.bufferedReader().use { it.readText().trim() } else null
        } finally { c.disconnect() }
    }

    private fun getAgents(cfg: Cfg, token: String): List<WzAgent> {
        val url = "https://${cfg.host}:$API_PORT/agents?limit=500&select=id,name,ip,status,lastKeepAlive"
        val c = open(url)
        c.requestMethod = "GET"
        c.setRequestProperty("Authorization", "Bearer $token")
        return try {
            if (c.responseCode != 200) return emptyList()
            val json = c.inputStream.bufferedReader().use { it.readText() }
            val items = JSONObject(json).getJSONObject("data").optJSONArray("affected_items") ?: return emptyList()
            val out = ArrayList<WzAgent>()
            for (i in 0 until items.length()) {
                val a = items.getJSONObject(i)
                out.add(
                    WzAgent(
                        a.optString("id"), a.optString("name", "?"),
                        a.optString("ip", "-"), a.optString("status", "?"),
                        a.optString("lastKeepAlive", "-")
                    )
                )
            }
            out
        } finally { c.disconnect() }
    }

    private fun getAlerts(cfg: Cfg, size: Int): List<WzAlert> {
        if (cfg.idxUser.isEmpty()) return emptyList()
        val url = "https://${cfg.host}:$IDX_PORT/wazuh-alerts-*/_search"
        val c = open(url)
        c.requestMethod = "POST"
        c.setRequestProperty("Authorization", basic(cfg.idxUser, cfg.idxPass))
        c.setRequestProperty("Content-Type", "application/json")
        c.doOutput = true
        val body = """{"size":$size,"sort":[{"timestamp":{"order":"desc"}}],"query":{"range":{"timestamp":{"gte":"now-24h"}}}}"""
        c.outputStream.use { it.write(body.toByteArray()) }
        return try {
            if (c.responseCode != 200) return emptyList()
            val json = c.inputStream.bufferedReader().use { it.readText() }
            val hits = JSONObject(json).getJSONObject("hits").optJSONArray("hits") ?: return emptyList()
            val out = ArrayList<WzAlert>()
            for (i in 0 until hits.length()) {
                val src = hits.getJSONObject(i).getJSONObject("_source")
                val rule = src.optJSONObject("rule")
                val agent = src.optJSONObject("agent")
                out.add(
                    WzAlert(
                        rule?.optString("description", "(alerte)") ?: "(alerte)",
                        rule?.optInt("level", 0) ?: 0,
                        agent?.optString("name", "-") ?: "-",
                        src.optString("timestamp", "")
                    )
                )
            }
            out
        } finally { c.disconnect() }
    }
}
