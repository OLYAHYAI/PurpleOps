package com.jbr.purpleops

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.InputStreamReader

/**
 * Execute des commandes en root via `su` (KernelSU).
 * - runStreaming : DANS le chroot Kali (outils offensifs).
 * - runAndroidRoot / readFileAsRoot : cote ANDROID (pm, cp, cat...).
 * Aucune dependance externe.
 */
object RootRunner {

    // Monte le chroot (idempotent) puis entre dedans en environnement vierge.
    private const val ENTER =
        "/data/local/kali-start.sh >/dev/null 2>&1; " +
        "env -i HOME=/root TERM=xterm-256color " +
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin " +
        "chroot /data/local/kali /bin/bash -lc"

    @Volatile
    private var current: Process? = null

    /**
     * Journalise une commande lancee dans un fichier lu par l'agent Wazuh
     * (`/data/local/kali/var/log/purpleops-runs.log` = `/android/kali/...` cote agent).
     * La ligne (deja au format syslog) est passee en base64 pour eviter tout
     * probleme de quoting shell.
     */
    suspend fun logRun(syslogLine: String) = withContext(Dispatchers.IO) {
        try {
            val b64 = android.util.Base64.encodeToString(
                (syslogLine + "\n").toByteArray(), android.util.Base64.NO_WRAP
            )
            ProcessBuilder(
                "su", "-c",
                "echo $b64 | base64 -d >> /data/local/kali/var/log/purpleops-runs.log"
            ).redirectErrorStream(true).start().waitFor()
        } catch (e: Exception) {
        }
    }

    /** Commande root cote Android (PAS dans le chroot). Retourne (code, sortie). */
    suspend fun runAndroidRoot(cmd: String): Pair<Int, String> = withContext(Dispatchers.IO) {
        try {
            val p = ProcessBuilder("su", "-c", cmd).redirectErrorStream(true).start()
            val out = p.inputStream.bufferedReader().readText()
            val code = p.waitFor()
            code to out
        } catch (e: Exception) {
            -1 to (e.message ?: "erreur")
        }
    }

    /** Lit un fichier cote Android en root (ex. presets.json externe). Null si absent. */
    suspend fun readFileAsRoot(path: String): String? = withContext(Dispatchers.IO) {
        try {
            val p = ProcessBuilder("su", "-c", "cat $path").start()
            val out = p.inputStream.bufferedReader().readText()
            if (p.waitFor() == 0 && out.isNotBlank()) out else null
        } catch (e: Exception) {
            null
        }
    }

    /** Lance `inner` DANS le chroot Kali et pousse chaque ligne de sortie vers onLine. */
    suspend fun runStreaming(inner: String, onLine: (String) -> Unit): Int =
        withContext(Dispatchers.IO) {
            val full = "$ENTER '$inner'"
            var code = -1
            try {
                val p = ProcessBuilder("su", "-c", full).redirectErrorStream(true).start()
                current = p
                val reader = BufferedReader(InputStreamReader(p.inputStream))
                var line: String?
                while (reader.readLine().also { line = it } != null) {
                    onLine(line!!)
                }
                code = p.waitFor()
            } catch (e: Exception) {
                onLine("[erreur] ${e.message}")
            } finally {
                current = null
            }
            code
        }

    fun stop() {
        current?.destroy()
        current = null
    }
}
