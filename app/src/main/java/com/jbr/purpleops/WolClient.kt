package com.jbr.purpleops

import android.content.Context
import android.util.Base64
import com.jcraft.jsch.ChannelExec
import com.jcraft.jsch.HostKey
import com.jcraft.jsch.HostKeyRepository
import com.jcraft.jsch.JSch
import com.jcraft.jsch.UserInfo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.net.Inet4Address
import java.net.NetworkInterface
import java.security.MessageDigest

/**
 * Réveil des nœuds Proxmox par wol-pi (Wake-on-LAN), depuis le téléphone.
 *
 * Réutilise l'outil existant du Pi, sans rien y ajouter : compte `wol`, commande
 * imposée `wake-request` (liste fermée proxmox | f4blox | pve), audit Wazuh
 * 100720-100727. Le téléphone joint wol-pi en pair Tailscale direct : il peut
 * donc réveiller F4BLOX (que n8n, hébergé dessus, ne peut pas réveiller) et
 * même les trois nœuds éteints.
 *
 * Sécurité : clé du téléphone dans le Keystore ([WolKey]) ; clé d'hôte du Pi
 * ÉPINGLÉE (ed25519), validée une fois par l'utilisateur dans Réglages.
 */
object WolClient {

    const val DEFAULT_HOST = "100.117.55.112"   // wol-pi (Tailscale)
    const val USER = "wol"
    const val P_HOST = "wol_host"
    const val P_HOSTKEY = "wol_hostkey"           // base64 du blob ed25519 épinglé
    val TARGETS = listOf("f4blox", "pve", "proxmox") // ordre de « Tout allumer » : le SOC d'abord
    private val RESULTS = setOf("sent", "already_up", "no_link", "error", "rejected")

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("purpleops", Context.MODE_PRIVATE)
    fun host(ctx: Context) = prefs(ctx).getString(P_HOST, DEFAULT_HOST).orEmpty().ifBlank { DEFAULT_HOST }
    fun pinnedKey(ctx: Context) = prefs(ctx).getString(P_HOSTKEY, "").orEmpty()
    fun isPaired(ctx: Context) = WolKey.exists() && pinnedKey(ctx).isNotEmpty()

    fun sshFingerprint(blob: ByteArray): String =
        "SHA256:" + Base64.encodeToString(MessageDigest.getInstance("SHA-256").digest(blob), Base64.NO_WRAP or Base64.NO_PADDING)

    /** IP Tailscale du téléphone (100.64.0.0/10), pour la restriction from= côté Pi. */
    fun phoneTailscaleIp(): String? = try {
        NetworkInterface.getNetworkInterfaces().toList().flatMap { it.inetAddresses.toList() }
            .filterIsInstance<Inet4Address>()
            .map { it.hostAddress ?: "" }
            .firstOrNull { ip -> ip.split(".").let { it.size == 4 && it[0] == "100" && (it[1].toIntOrNull() ?: 0) in 64..127 } }
    } catch (e: Exception) { null }

    /** Dépôt de clés d'hôte : n'accepte QUE la clé épinglée (ou capture pour l'appairage). */
    private class PinnedRepo(private val pinned: String?) : HostKeyRepository {
        var seenType: String? = null
        var seenKey: ByteArray? = null
        override fun check(host: String?, key: ByteArray?): Int {
            seenKey = key
            seenType = key?.let { typeOf(it) }
            if (pinned.isNullOrEmpty() || key == null) return HostKeyRepository.NOT_INCLUDED
            val p = Base64.decode(pinned, Base64.DEFAULT)
            return if (MessageDigest.isEqual(p, key)) HostKeyRepository.OK else HostKeyRepository.CHANGED
        }
        override fun add(hostkey: HostKey?, ui: UserInfo?) {}
        override fun remove(host: String?, type: String?) {}
        override fun remove(host: String?, type: String?, key: ByteArray?) {}
        override fun getKnownHostsRepositoryID(): String = "purpleops-pinned"
        override fun getHostKey(): Array<HostKey> = arrayOf()
        override fun getHostKey(host: String?, type: String?): Array<HostKey> = arrayOf()

        private fun typeOf(blob: ByteArray): String = try {
            val l = java.nio.ByteBuffer.wrap(blob).int
            String(blob, 4, l)
        } catch (e: Exception) { "?" }
    }

    private fun newSession(host: String, repo: HostKeyRepository): com.jcraft.jsch.Session {
        val jsch = JSch()
        jsch.hostKeyRepository = repo
        jsch.addIdentity(WolKey.KeystoreIdentity(), null)
        return jsch.getSession(USER, host, 22).apply {
            setConfig("StrictHostKeyChecking", "yes")
            setConfig("server_host_key", "ssh-ed25519")
            setConfig("PreferredAuthentications", "publickey")
            setConfig("PubkeyAcceptedAlgorithms", "ecdsa-sha2-nistp256")
            timeout = 12_000
        }
    }

    /**
     * Appairage, étape 1 : lit la clé d'hôte du Pi SANS lui faire confiance et
     * SANS s'authentifier (la connexion est coupée au contrôle de la clé).
     * Renvoie (type, base64, empreinte SHA256).
     */
    suspend fun probeHostKey(ctx: Context): Triple<String, String, String> = withContext(Dispatchers.IO) {
        WolKey.ensure()
        val repo = PinnedRepo(null)
        val s = newSession(host(ctx), repo)
        try { s.connect(12_000) } catch (e: Exception) { /* attendu : clé non épinglée */ } finally { s.disconnect() }
        val key = repo.seenKey ?: throw IllegalStateException("wol-pi injoignable en SSH (${host(ctx)}:22)")
        Triple(repo.seenType ?: "?", Base64.encodeToString(key, Base64.NO_WRAP), sshFingerprint(key))
    }

    fun pin(ctx: Context, b64: String) { prefs(ctx).edit().putString(P_HOSTKEY, b64).apply() }
    fun unpair(ctx: Context) { prefs(ctx).edit().remove(P_HOSTKEY).apply(); WolKey.delete() }

    /**
     * Demande le réveil d'une cible. Résultat = mot renvoyé par wol-pi
     * (sent | already_up | no_link | error | rejected) ou « ssh_error: … ».
     */
    suspend fun wake(ctx: Context, target: String): String = withContext(Dispatchers.IO) {
        require(target in TARGETS) { "cible hors liste" }
        val pinned = pinnedKey(ctx)
        if (pinned.isEmpty() || !WolKey.exists()) return@withContext "ssh_error: wol-pi non appairé (Réglages)"
        val s = newSession(host(ctx), PinnedRepo(pinned))
        try {
            s.connect(12_000)
            val ch = s.openChannel("exec") as ChannelExec
            ch.setCommand(target)
            val input = ch.inputStream
            ch.connect(10_000)
            val out = input.bufferedReader().readText().trim()
            ch.disconnect()
            out.lines().lastOrNull { it.trim() in RESULTS }?.trim() ?: "ssh_error: réponse inattendue « ${out.take(60)} »"
        } catch (e: Exception) {
            val msg = e.message.orEmpty()
            when {
                msg.contains("HostKey", true) -> "ssh_error: clé d'hôte de wol-pi DIFFÉRENTE de l'épinglée (refus)"
                msg.contains("Auth fail", true) -> "ssh_error: clé du téléphone refusée (ligne authorized_keys absente ?)"
                else -> "ssh_error: ${e.javaClass.simpleName} ${msg.take(80)}"
            }
        } finally { s.disconnect() }
    }

    /** Ligne de journal (agent Wazuh 027), même format que les commandes Kali. */
    private val sysFmt = java.text.SimpleDateFormat("MMM dd HH:mm:ss", java.util.Locale.US)
    suspend fun audit(target: String, result: String) {
        RootRunner.logRun("${sysFmt.format(java.util.Date())} localhost purpleops: cmd=wake $target result=${result.substringBefore(':')}")
    }
}
