package com.jbr.purpleops

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.BufferedReader
import java.io.InputStreamReader

/**
 * Execute des commandes dans le chroot Kali via `su` (KernelSU).
 * Aucune dependance externe : ProcessBuilder + flux ligne a ligne.
 */
object RootRunner {

    // Monte le chroot (idempotent) puis entre dedans en environnement vierge,
    // exactement comme kali-shell cote Termux.
    private const val ENTER =
        "/data/local/kali-start.sh >/dev/null 2>&1; " +
        "env -i HOME=/root TERM=xterm-256color " +
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin " +
        "chroot /data/local/kali /bin/bash -lc"

    @Volatile
    private var current: Process? = null

    /** Lit un fichier cote Android en root (pour un presets.json externe). Null si absent. */
    suspend fun readFileAsRoot(path: String): String? = withContext(Dispatchers.IO) {
        try {
            val p = ProcessBuilder("su", "-c", "cat $path")
                .redirectErrorStream(false)
                .start()
            val out = p.inputStream.bufferedReader().readText()
            val code = p.waitFor()
            if (code == 0 && out.isNotBlank()) out else null
        } catch (e: Exception) {
            null
        }
    }

    /**
     * Lance `inner` dans Kali et pousse chaque ligne de sortie vers onLine.
     * Bloquant : a appeler depuis un Dispatchers.IO.
     */
    suspend fun runStreaming(inner: String, onLine: (String) -> Unit): Int =
        withContext(Dispatchers.IO) {
            // inner est encadre de guillemets simples pour bash -lc.
            val full = "$ENTER '$inner'"
            var code = -1
            try {
                val p = ProcessBuilder("su", "-c", full)
                    .redirectErrorStream(true)
                    .start()
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
