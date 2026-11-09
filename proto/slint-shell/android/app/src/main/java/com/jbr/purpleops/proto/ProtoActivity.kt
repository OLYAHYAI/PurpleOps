package com.jbr.purpleops.proto

import android.app.NativeActivity
import android.content.Intent
import android.hardware.biometrics.BiometricManager.Authenticators.BIOMETRIC_STRONG
import android.hardware.biometrics.BiometricPrompt
import android.os.CancellationSignal
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import java.io.File
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.Signature
import java.security.spec.ECGenParameterSpec

/**
 * Coquille du prototype : NativeActivity (Slint dessine tout) sous-classée en
 * Kotlin pour garder les API Android. Les méthodes publiques sont appelées par
 * Rust via JNI sur cet objet ; les `external` rappellent Rust.
 *
 * Aucune dépendance AndroidX : BiometricPrompt du FRAMEWORK (API 28+), qui
 * fonctionne avec n'importe quelle Activity (pas besoin de FragmentActivity).
 */
class ProtoActivity : NativeActivity() {

    companion object {
        // NativeActivity charge la bibliothèque par dlopen ; System.loadLibrary
        // l'enregistre aussi auprès de la JVM pour résoudre les `external`.
        init { System.loadLibrary("proto") }
        private const val KEY_ALIAS = "proto.power.v1"
        private const val REQ_WEB = 42
    }

    external fun nativeOnAuthResult(ok: Boolean, detail: String)
    external fun nativeOnWebReturned()

    // ------------------------------------------------ appelées depuis Rust

    /** Taille de l'APK installé (critère 5). */
    fun apkSize(): Long = File(applicationInfo.sourceDir).length()

    /** Ouvre une activité Kotlin classique (critère 4). */
    fun openWeb() {
        runOnUiThread {
            @Suppress("DEPRECATION")
            startActivityForResult(Intent(this, WebActivity::class.java), REQ_WEB)
        }
    }

    /** Biométrie forte + signature Keystore liée à l'empreinte (critère 3). */
    fun authenticate() {
        runOnUiThread {
            try {
                val key = loadOrCreateKey()
                val sig = Signature.getInstance("SHA256withECDSA").apply { initSign(key) }
                val prompt = BiometricPrompt.Builder(this)
                    .setTitle("Prototype PurpleOps")
                    .setSubtitle("Signer un nonce de test (aucune action réelle)")
                    .setAllowedAuthenticators(BIOMETRIC_STRONG)
                    .setNegativeButton("Annuler", mainExecutor) { _, _ ->
                        nativeOnAuthResult(false, "Annulé par l'utilisateur.")
                    }
                    .build()
                prompt.authenticate(
                    BiometricPrompt.CryptoObject(sig), CancellationSignal(), mainExecutor,
                    object : BiometricPrompt.AuthenticationCallback() {
                        override fun onAuthenticationSucceeded(r: BiometricPrompt.AuthenticationResult) {
                            try {
                                val s = r.cryptoObject!!.signature!!
                                s.update(ByteArray(32).also { SecureRandom().nextBytes(it) })
                                val out = s.sign()
                                nativeOnAuthResult(true, "Signature ECDSA P-256 de ${out.size} octets, ${where(key)}.")
                            } catch (e: Exception) {
                                nativeOnAuthResult(false, "Signature refusée : ${e.javaClass.simpleName} ${e.message}")
                            }
                        }
                        override fun onAuthenticationError(code: Int, msg: CharSequence) {
                            nativeOnAuthResult(false, "Erreur biométrie $code : $msg")
                        }
                    })
            } catch (e: Exception) {
                nativeOnAuthResult(false, "${e.javaClass.simpleName} : ${e.message}")
            }
        }
    }

    // ------------------------------------------------ interne

    @Deprecated("API Activity")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ_WEB) nativeOnWebReturned()
    }

    /** Clé non exportable, StrongBox si dispo, empreinte exigée à CHAQUE signature. */
    private fun loadOrCreateKey(): PrivateKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(KEY_ALIAS, null) as? PrivateKey)?.let { return it }
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(KEY_ALIAS, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            .setInvalidatedByBiometricEnrollment(true)
            .setIsStrongBoxBacked(strongBox)
            .build()
        val gen = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        try {
            gen.initialize(spec(true)); gen.generateKeyPair()
        } catch (e: StrongBoxUnavailableException) {
            gen.initialize(spec(false)); gen.generateKeyPair()
        }
        return ks.getKey(KEY_ALIAS, null) as PrivateKey
    }

    private fun where(key: PrivateKey): String = try {
        val info = KeyFactory.getInstance(key.algorithm, "AndroidKeyStore").getKeySpec(key, KeyInfo::class.java)
        when (info.securityLevel) {
            KeyProperties.SECURITY_LEVEL_STRONGBOX -> "clé en StrongBox (Titan M2)"
            KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "clé en TEE"
            else -> "niveau de sécurité ${info.securityLevel}"
        }
    } catch (e: Exception) { "niveau inconnu" }
}
