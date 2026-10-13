package com.jbr.purpleops

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Auto-update silencieuse (profite du root).
 * 1. interroge l'API GitHub Releases du repo (BuildConfig.GITHUB_REPO) ;
 * 2. compare le versionCode de la derniere release a celui installe ;
 * 3. si plus recent : telecharge l'APK, l'installe en silencieux via
 *    `su -c pm install -r`, et notifie.
 *
 * Tout est best-effort : hors-ligne ou erreur => on ne fait rien (pas de crash).
 */
object Updater {

    private const val CHANNEL_ID = "updates"
    private const val NOTIF_ID = 42

    data class Release(val code: Int, val name: String, val apkUrl: String)

    suspend fun checkAndUpdate(ctx: Context, log: (String) -> Unit) {
        try {
            val rel = fetchLatest() ?: return
            val installed = BuildConfig.VERSION_CODE
            if (rel.code <= installed) {
                log("[maj] a jour (installee v$installed, derniere v${rel.code})")
                return
            }
            log("[maj] nouvelle version v${rel.code} detectee (installee v$installed) — telechargement…")
            notify(ctx, "PurpleOps — mise a jour", "Installation de la v${rel.code} en cours…")

            val apk = download(ctx, rel.apkUrl) ?: run {
                log("[maj] echec du telechargement")
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
            log("[maj] erreur : ${e.message}")
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
            var apkUrl: String? = null
            for (i in 0 until assets.length()) {
                val a = assets.getJSONObject(i)
                if (a.optString("name").endsWith(".apk")) {
                    apkUrl = a.optString("browser_download_url"); break
                }
            }
            apkUrl?.let { Release(code, tag, it) }
        } finally {
            c.disconnect()
        }
    }

    private suspend fun download(ctx: Context, url: String): File? = withContext(Dispatchers.IO) {
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            instanceFollowRedirects = true
            connectTimeout = 15000
            readTimeout = 60000
            setRequestProperty("User-Agent", "PurpleOps-Updater")
        }
        try {
            if (c.responseCode !in 200..299) return@withContext null
            val out = File(ctx.cacheDir, "purpleops-update.apk")
            c.inputStream.use { input -> out.outputStream().use { input.copyTo(it) } }
            out
        } catch (e: Exception) {
            null
        } finally {
            c.disconnect()
        }
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
