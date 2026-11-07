package com.jbr.purpleops

import android.content.ClipData
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.view.InputEvent
import android.view.KeyCharacterMap
import android.view.KeyEvent

/**
 * Aide de l'Écran USB, lancée par screenctl via `app_process` (uid shell), hors de
 * l'appli : elle parle directement aux services Android, comme le serveur de scrcpy.
 *
 * - Presse-papiers : lecture / écriture via le service « clipboard » (IClipboard).
 * - Texte Unicode : frappe par la table de caractères du clavier virtuel (quelle que
 *   soit la disposition) ; pour un caractère sans touche (é, emoji…), collage via le
 *   presse-papiers, puis restauration de son contenu précédent.
 *
 * Protocole ligne à ligne, textes en base64 (UTF-8) :
 *   entrée  « T <b64> » taper ce texte    « C <b64> » définir le presse-papiers
 *   sortie  « R » prête   « C <b64> » presse-papiers changé sur le téléphone   « E <msg> »
 *
 * Les API cachées changent d'une version d'Android à l'autre : les méthodes sont
 * trouvées par leur nom et leurs paramètres remplis selon leur type.
 */
object ScreenHelper {
    private const val PKG = "com.android.shell"
    private val lock = Object()

    @Volatile
    private var lastClip: String? = null
    private lateinit var clipboard: Any
    private var injector: Pair<Any, java.lang.reflect.Method>? = null

    @JvmStatic
    fun main(args: Array<String>) {
        Looper.prepare()
        try {
            val sm = Class.forName("android.os.ServiceManager")
            val binder = sm.getMethod("getService", String::class.java).invoke(null, "clipboard") as IBinder
            clipboard = Class.forName("android.content.IClipboard\$Stub")
                .getMethod("asInterface", IBinder::class.java).invoke(null, binder)!!
        } catch (e: Throwable) {
            emit("E presse-papiers indisponible : ${e.cause ?: e}")
        }
        injector = try {
            val mgr = try {
                Class.forName("android.hardware.input.InputManagerGlobal").getMethod("getInstance").invoke(null)
            } catch (_: Throwable) {
                Class.forName("android.hardware.input.InputManager").getMethod("getInstance").invoke(null)
            }!!
            mgr to mgr.javaClass.getMethod("injectInputEvent", InputEvent::class.java, Int::class.javaPrimitiveType)
        } catch (e: Throwable) {
            emit("E injection de texte indisponible : ${e.cause ?: e}")
            null
        }
        lastClip = runCatching { getClip() }.getOrNull()

        // Surveillance du presse-papiers du téléphone (pas de rappel accessible hors appli).
        Thread {
            while (true) {
                SystemClock.sleep(400)
                val now = runCatching { getClip() }.getOrNull() ?: continue
                synchronized(lock) {
                    if (now != lastClip) {
                        lastClip = now
                        emit("C " + b64(now))
                    }
                }
            }
        }.apply { isDaemon = true }.start()

        emit("R")
        val input = System.`in`.bufferedReader()
        while (true) {
            val line = input.readLine() ?: break
            val text = runCatching { String(Base64.decode(line.substring(2), Base64.NO_WRAP), Charsets.UTF_8) }.getOrNull()
                ?: continue
            try {
                when (line[0]) {
                    'C' -> synchronized(lock) {
                        lastClip = text
                        setClip(text)
                    }
                    'T' -> type(text)
                }
            } catch (e: Throwable) {
                emit("E ${line[0]} : ${e.cause ?: e}")
            }
        }
    }

    private fun emit(s: String) = synchronized(System.out) {
        System.out.println(s)
        System.out.flush()
    }

    private fun b64(s: String) = Base64.encodeToString(s.toByteArray(Charsets.UTF_8), Base64.NO_WRAP)

    /** Appelle une méthode d'IClipboard en remplissant ses paramètres selon leur type. */
    private fun callClip(name: String, clip: ClipData?): Any? {
        val m = clipboard.javaClass.methods.filter { it.name == name }.maxByOrNull { it.parameterTypes.size }
            ?: throw NoSuchMethodException(name)
        var strings = 0
        val args = m.parameterTypes.map { t ->
            when {
                t == ClipData::class.java -> clip
                t == String::class.java -> if (strings++ == 0) PKG else null // paquet, puis attributionTag
                t == Int::class.javaPrimitiveType -> 0 // userId, deviceId
                t == Boolean::class.javaPrimitiveType -> false
                else -> null
            }
        }
        return m.invoke(clipboard, *args.toTypedArray())
    }

    private fun getClip(): String? {
        val c = callClip("getPrimaryClip", null) as ClipData? ?: return null
        if (c.itemCount == 0) return null
        val item = c.getItemAt(0)
        return (item.text ?: item.htmlText)?.toString()
    }

    private fun setClip(text: String) {
        callClip("setPrimaryClip", ClipData.newPlainText("PurpleOps", text))
    }

    private fun inject(e: KeyEvent) {
        val (mgr, m) = injector ?: return
        m.invoke(mgr, e, 0) // INJECT_INPUT_EVENT_MODE_ASYNC
    }

    private fun key(code: Int) {
        val t = SystemClock.uptimeMillis()
        inject(KeyEvent(t, t, KeyEvent.ACTION_DOWN, code, 0))
        inject(KeyEvent(t, t, KeyEvent.ACTION_UP, code, 0))
    }

    /** Tape le texte : touches du clavier virtuel quand elles existent, collage sinon. */
    private fun type(text: String) {
        val kcm = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD)
        val pending = StringBuilder()
        fun flushPaste() {
            if (pending.isEmpty()) return
            synchronized(lock) {
                val before = runCatching { getClip() }.getOrNull()
                lastClip = pending.toString()
                setClip(pending.toString())
                key(KeyEvent.KEYCODE_PASTE)
                SystemClock.sleep(80)
                if (before != null) {
                    lastClip = before
                    setClip(before)
                }
            }
            pending.clear()
        }
        for (ch in text) {
            val evs = kcm.getEvents(charArrayOf(ch))
            if (evs == null) {
                pending.append(ch)
                continue
            }
            flushPaste()
            val now = SystemClock.uptimeMillis()
            for (e in evs) inject(KeyEvent.changeTimeRepeat(e, now, 0))
        }
        flushPaste()
    }
}
