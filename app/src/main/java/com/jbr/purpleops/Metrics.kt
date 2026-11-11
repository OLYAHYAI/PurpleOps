package com.jbr.purpleops

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.Socket
import java.net.URL

/**
 * Données de l'Accueil : `/metrics.json` de soc-display (un collecteur, deux
 * affichages ; schéma v1, lecture seule, tailnet uniquement, HTTPS Let's
 * Encrypt via tailscale serve → magasin de certificats système).
 */
data class MNode(
    val name: String, val online: Boolean, val cpu: Double, val maxcpu: Int,
    val mem: Long, val maxmem: Long, val uptime: Long, val guestsRunning: Int, val guestsTotal: Int
)
data class MAlert(val ts: String?, val level: Int, val agent: String, val desc: String)
data class MLink(val name: String, val state: Int)
data class MTarget(val name: String, val lastMs: Double?)

data class Metrics(
    val collectedAt: String?,
    val level: Int, val levelText: String, val reasons: List<String>,
    val quorate: Boolean?, val nodes: List<MNode>,
    val crit: Int, val high: Int, val total24h: Long, val recent: List<MAlert>,
    val agentsActive: Int, val agentsTotal: Int, val agentsDown: List<String>,
    val powerW: Double?, val dayAvgW: Double?,
    val chain: List<MLink>, val n8n24h: Long, val ia24h: Long,
    val mitreCovered: Int, val mitreTotal: Int, val blindSpots: List<String>,
    val net: List<MTarget>, val dnsOk: Boolean?, val dnsMs: Double?,
)

/**
 * Machines réveillables : liste FIXE embarquée (l'Énergie doit marcher quand tout est éteint).
 * [ip] : IP Tailscale pour les nœuds du cluster ; IP LAN (routes de sous-réseau pfSense/F4BLOX)
 * pour PBS, qui n'a volontairement pas Tailscale (serveur de sauvegarde isolé).
 * [port] : interface web sondée et ouverte (8006 PVE, 8007 PBS).
 * [inCluster] : false pour PBS (hors cluster, en veille la plupart du temps) → pas dans « Tout allumer ».
 */
data class NodeDef(
    val target: String, val pveName: String, val ip: String, val role: String, val via: String,
    val port: Int = 8006, val inCluster: Boolean = true,
)

object Cluster {
    val NODES = listOf(
        NodeDef("f4blox", "F4BLOX", "100.98.171.60", "SOC · Wazuh, n8n, Paperclip", "wlan0"),
        NodeDef("pve", "pve", "100.64.189.9", "Velociraptor, nrj-hub", "wlan0"),
        NodeDef("proxmox", "proxmox", "100.123.155.93", "GPU · soc-display, Zeek", "eth0"),
        NodeDef("pbs", "BU (PBS)", "10.10.10.222", "Sauvegarde · Proxmox Backup Server", "wlan0", port = 8007, inCluster = false),
    )
    /** Les 3 nœuds du cluster (« Tout allumer »). */
    val CLUSTER = NODES.filter { it.inCluster }
    fun proxmoxUrl(n: NodeDef) = "https://${n.ip}:${n.port}/"

    /** Sonde directe : l'interface web répond-elle (8006 PVE, 8007 PBS) ? */
    suspend fun probe(): Map<String, Boolean> = coroutineScope {
        NODES.map { n ->
            async(Dispatchers.IO) {
                n.target to try {
                    Socket().use { it.connect(InetSocketAddress(n.ip, n.port), 3000) }; true
                } catch (e: Exception) { false }
            }
        }.awaitAll().toMap()
    }
}

object MetricsClient {
    const val P_URL = "metrics_url"
    const val DEFAULT_URL = "https://soc-display.tail8a4866.ts.net/metrics.json"

    fun url(ctx: Context) = ctx.getSharedPreferences("purpleops", Context.MODE_PRIVATE)
        .getString(P_URL, DEFAULT_URL).orEmpty().ifBlank { DEFAULT_URL }

    suspend fun load(ctx: Context): Result<Metrics> = withContext(Dispatchers.IO) {
        try {
            val c = URL(url(ctx)).openConnection() as HttpURLConnection
            c.connectTimeout = 5000; c.readTimeout = 8000
            c.setRequestProperty("Accept", "application/json")
            try {
                if (c.responseCode != 200) return@withContext Result.failure(IllegalStateException("HTTP ${c.responseCode}"))
                Result.success(parse(JSONObject(c.inputStream.bufferedReader().readText())))
            } finally { c.disconnect() }
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    private fun JSONObject.optObj(k: String): JSONObject? = if (isNull(k)) null else optJSONObject(k)
    private fun JSONObject.optD(k: String): Double? = if (isNull(k) || !has(k)) null else optDouble(k)
    private fun JSONArray?.strings(): List<String> = if (this == null) emptyList() else (0 until length()).map { optString(it) }

    fun parse(j: JSONObject): Metrics {
        require(j.optInt("schema") == 1) { "schéma /metrics.json inattendu (${j.optInt("schema")})" }
        val ov = j.getJSONObject("overall")
        val pve = j.optObj("pve")
        val nodes = pve?.optJSONArray("nodes")?.let { a -> (0 until a.length()).map { i -> a.getJSONObject(i).let { n ->
            MNode(n.optString("name"), n.optBoolean("online"), n.optDouble("cpu", 0.0), n.optInt("maxcpu"),
                n.optLong("mem"), n.optLong("maxmem"), n.optLong("uptime"), n.optInt("guests_running"), n.optInt("guests_total"))
        } } } ?: emptyList()
        val al = j.optObj("alerts")
        val recent = al?.optJSONArray("recent")?.let { a -> (0 until a.length()).map { i -> a.getJSONObject(i).let {
            MAlert(if (it.isNull("ts")) null else it.optString("ts"), it.optInt("level"), it.optString("agent"), it.optString("desc"))
        } } } ?: emptyList()
        val ag = j.optObj("agents")
        val en = j.optObj("energy")
        val so = j.optObj("soar")
        val chain = so?.optJSONArray("chain")?.let { a -> (0 until a.length()).map { i -> a.getJSONObject(i).let { MLink(it.optString("name"), it.optInt("state")) } } } ?: emptyList()
        val mi = j.optObj("mitre")
        val ne = j.optObj("net")
        val targets = ne?.optJSONArray("targets")?.let { a -> (0 until a.length()).map { i -> a.getJSONObject(i).let { MTarget(it.optString("name"), it.optD("last_ms")) } } } ?: emptyList()
        return Metrics(
            collectedAt = if (j.isNull("collected_at")) null else j.optString("collected_at"),
            level = ov.optInt("level"), levelText = ov.optString("text"), reasons = ov.optJSONArray("reasons").strings(),
            quorate = pve?.let { if (it.isNull("quorate")) null else it.optBoolean("quorate") }, nodes = nodes,
            crit = al?.optInt("crit") ?: 0, high = al?.optInt("high") ?: 0,
            total24h = so?.optLong("total_24h") ?: 0, recent = recent,
            agentsActive = ag?.optInt("active") ?: 0, agentsTotal = ag?.optInt("total") ?: 0, agentsDown = ag?.optJSONArray("disconnected").strings(),
            powerW = en?.optD("power_w"), dayAvgW = en?.optD("day_avg_w"),
            chain = chain, n8n24h = so?.optLong("n8n_24h") ?: 0, ia24h = so?.optLong("ia_24h") ?: 0,
            mitreCovered = mi?.optInt("covered") ?: 0, mitreTotal = mi?.optInt("total") ?: 14, blindSpots = mi?.optJSONArray("blind_spots").strings(),
            net = targets, dnsOk = ne?.let { if (it.isNull("dns_ok")) null else it.optBoolean("dns_ok") }, dnsMs = ne?.optD("dns_ms"),
        )
    }
}
