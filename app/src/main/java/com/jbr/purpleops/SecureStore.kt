package com.jbr.purpleops

import android.content.Context
import android.content.SharedPreferences
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Secrets de l'appli (mots de passe Wazuh…) chiffrés au repos.
 *
 * Clé AES-256-GCM générée DANS l'Android Keystore (non exportable, liée à
 * l'appli et donc à sa clé de signature). Les valeurs sont rangées dans les
 * SharedPreferences sous la forme `enc:v1:<base64(iv | chiffré+tag)>`.
 * Pas de biométrie ici : le tableau de bord doit pouvoir se rafraîchir seul.
 * La clé d'extinction (rubrique Énergie) aura sa propre clé, liée à l'empreinte.
 */
object SecureStore {

    private const val KEY_ALIAS = "purpleops.secrets.v1"
    private const val PREFIX = "enc:v1:"
    private const val PREFS = "purpleops"
    private const val GCM_TAG_BITS = 128

    /** Clés de préférences qui contiennent un secret. */
    val SECRET_KEYS = listOf("wz_api_pass", "wz_idx_pass")

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setRandomizedEncryptionRequired(true)
                .build()
        )
        return gen.generateKey()
    }

    private fun encrypt(plain: String): String {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, key())
        val iv = c.iv
        val ct = c.doFinal(plain.toByteArray(Charsets.UTF_8))
        return PREFIX + Base64.encodeToString(iv + ct, Base64.NO_WRAP)
    }

    private fun decrypt(stored: String): String? = try {
        val raw = Base64.decode(stored.removePrefix(PREFIX), Base64.NO_WRAP)
        val iv = raw.copyOfRange(0, 12)
        val ct = raw.copyOfRange(12, raw.size)
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(GCM_TAG_BITS, iv))
        String(c.doFinal(ct), Charsets.UTF_8)
    } catch (e: Exception) {
        null // clé perdue (réinstallation) ou donnée altérée : le secret est à ressaisir
    }

    private fun prefs(ctx: Context): SharedPreferences =
        ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /** Lit un secret ; "" s'il est absent ou indéchiffrable. */
    fun get(ctx: Context, name: String): String {
        val v = prefs(ctx).getString(name, null) ?: return ""
        if (!v.startsWith(PREFIX)) return v // valeur héritée en clair : migrée par migrate()
        return decrypt(v).orEmpty()
    }

    fun put(ed: SharedPreferences.Editor, name: String, value: String) {
        if (value.isEmpty()) ed.remove(name) else ed.putString(name, encrypt(value))
    }

    /**
     * Chiffre les secrets encore stockés en clair par les versions précédentes.
     * Idempotent ; appelé au démarrage.
     */
    fun migrate(ctx: Context) {
        val p = prefs(ctx)
        val ed = p.edit()
        var changed = false
        for (k in SECRET_KEYS) {
            val v = p.getString(k, null) ?: continue
            if (!v.startsWith(PREFIX)) {
                put(ed, k, v); changed = true
            }
        }
        if (changed) ed.commit()
    }
}
