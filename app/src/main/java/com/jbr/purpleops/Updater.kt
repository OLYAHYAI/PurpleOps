package com.jbr.purpleops

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import android.content.pm.PackageManager
import java.io.File
import java.security.MessageDigest
import java.net.HttpURLConnection
import java.net.URL

/**
 * Auto-update silencieuse (profite du root).
 * 1. interroge l'API GitHub Releases du repo (BuildConfig.GITHUB_REPO) ;
 * 2. compare le versionCode de la derniere release a celui installe ;
 * 3. si plus recent : telecharge l'APK et son empreinte `<apk>.sha256` publiee
 *    par la CI, verifie l'empreinte (fail-closed : pas d'empreinte = pas
 *    d'installation) PUIS que l'APK est signe par la meme cle que l'appli
 *    installee, l'installe en silencieux via `su -c pm install -r`, et notifie.
 *
 * Tout est best-effort : hors-ligne ou erreur => on ne fait rien (pas de crash).
 */
object Updater {

    private const val CHANNEL_ID = "updates"
    private const val NOTIF_ID = 42

    data class Release(val code: Int, val name: String, val apkUrl: String, val size: Long, val shaUrl: String?)

    // Une seule mise à jour à la fois : la vérification au démarrage et le bouton
    // « Chercher une mise à jour » écrivaient sinon le même fichier en même temps.
    private val busy = kotlinx.coroutines.sync.Mutex()

    suspend fun checkAndUpdate(ctx: Context, log: (String) -> Unit) {
        if (!busy.tryLock()) {
            log("[maj] une vérification est déjà en cours…")
            return
        }
        try {
            val rel = fetchLatest() ?: return
            val installed = BuildConfig.VERSION_CODE
            if (rel.code <= installed) {
                log("[maj] a jour (installee v$installed, derniere v${rel.code})")
                return
            }
            log("[maj] nouvelle version v${rel.code} detectee (installee v$installed) — telechargement…")
            notify(ctx, "PurpleOps — mise a jour", "Installation de la v${rel.code} en cours…")

            var apk: File? = null
            for (attempt in 1..3) {
                val (f, err) = download(ctx, rel.apkUrl, rel.size)
                if (f != null) {
                    apk = f
                    break
                }
                log("[maj] echec du telechargement (essai $attempt/3) : $err")
                if (attempt < 3) kotlinx.coroutines.delay(3000)
            }
            if (apk == null) return
            val why = verify(ctx, apk, rel)
            if (why != null) {
                apk.delete()
                log("[maj] REFUS d'installer v${rel.code} : $why")
                notify(ctx, "PurpleOps — mise a jour refusee", why)
                return
            }
            log("[maj] installation silencieuse (root)…")
            val (code, out) = RootRunner.runAndroidRoot(
                "cp '${apk.absolutePath}' /data/local/tmp/po-update.apk && " +
                "chmod 644 /data/local/tmp/po-update.apk && " +
                "pm install -r /data/local/tmp/po-update.apk; " +
                "rm -f /data/local/tmp/po-update.apk"
            )
            // Note : pm install -r remplace l'app en cours => le process peut etre tue ici.
            if (out.contains("Success")) {
                log("[maj] v${rel.code} installee.")
            } else {
                log("[maj] echec install (code $code) : ${out.trim()}")
            }
        } catch (e: Exception) {
            log("[maj] erreur : ${e.javaClass.simpleName} ${e.message}")
        } finally {
            busy.unlock()
        }
    }

    private suspend fun fetchLatest(): Release? = withContext(Dispatchers.IO) {
        val url = "https://api.github.com/repos/${BuildConfig.GITHUB_REPO}/releases/latest"
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "GET"
            connectTimeout = 10000
            readTimeout = 15000
            setRequestProperty("Accept", "application/vnd.github+json")
            setRequestProperty("User-Agent", "PurpleOps-Updater")
        }
        try {
            if (c.responseCode != 200) return@withContext null
            val json = c.inputStream.bufferedReader().use { it.readText() }
            val o = JSONObject(json)
            val tag = o.optString("tag_name", o.optString("name", ""))
            val code = Regex("\\d+").find(tag)?.value?.toIntOrNull() ?: return@withContext null
            val assets = o.optJSONArray("assets") ?: return@withContext null
            var apkName: String? = null
            var apkUrl: String? = null
            var size = -1L
            for (i in 0 until assets.length()) {
                val a = assets.getJSONObject(i)
                if (a.optString("name").endsWith(".apk")) {
                    apkName = a.optString("name")
                    apkUrl = a.optString("browser_download_url")
                    size = a.optLong("size", -1L)
                    break
                }
            }
            var shaUrl: String? = null
            for (i in 0 until assets.length()) {
                val a = assets.getJSONObject(i)
                if (apkName != null && a.optString("name") == "$apkName.sha256") shaUrl = a.optString("browser_download_url")
            }
            apkUrl?.let { Release(code, tag, it, size, shaUrl) }
        } finally {
            c.disconnect()
        }
    }

    /**
     * Télécharge l'APK dans un fichier temporaire puis le renomme, après avoir
     * vérifié sa taille. Renvoie (fichier, null) ou (null, raison lisible).
     */
    private suspend fun download(ctx: Context, url: String, size: Long): Pair<File?, String?> =
        withContext(Dispatchers.IO) {
            val c = (URL(url).openConnection() as HttpURLConnection).apply {
                instanceFollowRedirects = true
                connectTimeout = 15000
                readTimeout = 60000
                setRequestProperty("User-Agent", "PurpleOps-Updater")
            }
            try {
                val code = c.responseCode
                if (code !in 200..299) return@withContext null to "HTTP $code (${c.url.host})"
                val part = File(ctx.cacheDir, "purpleops-update.apk.part")
                val out = File(ctx.cacheDir, "purpleops-update.apk")
                c.inputStream.use { input -> part.outputStream().use { input.copyTo(it) } }
                if (size > 0 && part.length() != size) {
                    part.delete()
                    return@withContext null to "fichier incomplet (${part.length()} / $size octets)"
                }
                out.delete()
                if (!part.renameTo(out)) return@withContext null to "renommage impossible"
                out to null
            } catch (e: Exception) {
                null to "${e.javaClass.simpleName} : ${e.message} (${c.url.host})"
            } finally {
                c.disconnect()
            }
        }

    /** null si l'APK est acceptable, sinon la raison du refus. */
    private suspend fun verify(ctx: Context, apk: File, rel: Release): String? = withContext(Dispatchers.IO) {
        val shaUrl = rel.shaUrl ?: return@withContext "empreinte .sha256 absente de la release"
        val expected = try {
            val c = (URL(shaUrl).openConnection() as HttpURLConnection).apply {
                instanceFollowRedirects = true; connectTimeout = 15000; readTimeout = 15000
                setRequestProperty("User-Agent", "PurpleOps-Updater")
            }
            try {
                if (c.responseCode !in 200..299) return@withContext "empreinte illisible (HTTP ${c.responseCode})"
                c.inputStream.bufferedReader().use { it.readText() }.trim().split(Regex("\\s+")).first().lowercase()
            } finally { c.disconnect() }
        } catch (e: Exception) { return@withContext "empreinte illisible (${e.javaClass.simpleName})" }
        if (!Regex("^[0-9a-f]{64}$").matches(expected)) return@withContext "empreinte mal formee"
        val md = MessageDigest.getInstance("SHA-256")
        apk.inputStream().use { input ->
            val buf = ByteArray(1 shl 16)
            while (true) { val n = input.read(buf); if (n < 0) break; md.update(buf, 0, n) }
        }
        val actual = md.digest().joinToString("") { "%02x".format(it) }
        if (actual != expected) return@withContext "empreinte SHA-256 differente (APK altere ou incomplet)"

        // Meme cle de signature que l'appli installee ? (pm le refuserait de toute
        // facon ; on le verifie avant pour un message clair et sans passer par root.)
        val pm = ctx.packageManager
        val flags = PackageManager.GET_SIGNING_CERTIFICATES
        val mine = pm.getPackageInfo(ctx.packageName, flags).signingInfo?.apkContentsSigners
        val theirs = pm.getPackageArchiveInfo(apk.absolutePath, flags)?.signingInfo?.apkContentsSigners
        if (mine.isNullOrEmpty() || theirs.isNullOrEmpty()) return@withContext "signature de l'APK illisible"
        fun fp(s: Array<android.content.pm.Signature>) =
            s.map { MessageDigest.getInstance("SHA-256").digest(it.toByteArray()).joinToString("") { b -> "%02x".format(b) } }.toSet()
        if (fp(mine) != fp(theirs)) return@withContext "APK signe par une autre cle que l'appli installee"
        null
    }

    private fun notify(ctx: Context, title: String, text: String) {
        try {
            val nm = ctx.getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Mises a jour", NotificationManager.IMPORTANCE_DEFAULT)
            )
            val n = NotificationCompat.Builder(ctx, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setContentTitle(title)
                .setContentText(text)
                .setAutoCancel(true)
                .build()
            NotificationManagerCompat.from(ctx).notify(NOTIF_ID, n)
        } catch (e: Exception) {
            // notification best-effort
        }
    }
}
