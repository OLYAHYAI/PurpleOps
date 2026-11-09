package com.jbr.purpleops

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import com.jcraft.jsch.Identity
import java.io.ByteArrayOutputStream
import java.io.DataOutputStream
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec

/**
 * Clé SSH du téléphone pour le compte `wol` de wol-pi (réveil des nœuds).
 *
 * - ECDSA P-256 générée DANS l'Android Keystore : la clé privée n'en sort
 *   jamais (StrongBox si disponible). Pas d'authentification utilisateur
 *   exigée : le téléphone n'a pas de verrouillage d'écran sécurisé (décision
 *   du 09/11/2026), et la clé ne permet que le réveil (commande imposée côté Pi).
 * - JSch signe via [KeystoreIdentity] : l'encodage SSH (RFC 5656) est fait ici.
 */
object WolKey {

    const val ALIAS = "purpleops.wol.v1"
    private const val ALG = "ecdsa-sha2-nistp256"
    private const val CURVE = "nistp256"

    private fun ks() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

    fun exists(): Boolean = ks().containsAlias(ALIAS)

    /** Crée la paire si absente. */
    fun ensure() {
        if (exists()) return
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setIsStrongBoxBacked(strongBox)
            .build()
        val gen = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore")
        try {
            gen.initialize(spec(true)); gen.generateKeyPair()
        } catch (e: Exception) {
            // StrongBox indisponible : TEE (toujours non exportable)
            gen.initialize(spec(false)); gen.generateKeyPair()
        }
    }

    /** Supprime la clé (ré-appairage). */
    fun delete() { if (exists()) ks().deleteEntry(ALIAS) }

    private fun privateKey(): PrivateKey = ks().getKey(ALIAS, null) as PrivateKey
    private fun publicKey(): ECPublicKey = ks().getCertificate(ALIAS).publicKey as ECPublicKey

    private fun sshString(out: DataOutputStream, b: ByteArray) { out.writeInt(b.size); out.write(b) }
    private fun sshMpint(out: DataOutputStream, v: BigInteger) = sshString(out, v.toByteArray())

    private fun fixed32(v: BigInteger): ByteArray {
        val b = v.toByteArray()
        return when {
            b.size == 32 -> b
            b.size > 32 -> b.copyOfRange(b.size - 32, b.size)
            else -> ByteArray(32 - b.size) + b
        }
    }

    /** Blob de clé publique SSH : string alg, string courbe, string point non compressé. */
    fun publicBlob(): ByteArray {
        val w = publicKey().w
        val point = byteArrayOf(0x04) + fixed32(w.affineX) + fixed32(w.affineY)
        val bos = ByteArrayOutputStream()
        DataOutputStream(bos).use { o ->
            sshString(o, ALG.toByteArray()); sshString(o, CURVE.toByteArray()); sshString(o, point)
        }
        return bos.toByteArray()
    }

    /**
     * Ligne `authorized_keys` complète à coller sur le Pi : mêmes restrictions
     * que la clé n8n (commande imposée, source unique = IP Tailscale du téléphone).
     */
    fun authorizedKeysLine(phoneTailscaleIp: String): String =
        "restrict,from=\"$phoneTailscaleIp\",command=\"/usr/local/bin/wake-request\" " +
            "$ALG ${Base64.encodeToString(publicBlob(), Base64.NO_WRAP)} purpleops-wake"

    /** Empreinte SHA256 au format OpenSSH (comme `ssh-keygen -lf`). */
    fun fingerprint(): String = WolClient.sshFingerprint(publicBlob())

    /**
     * Signature SSH : string alg + string(mpint r + mpint s).
     * Java produit du DER (SEQUENCE { INTEGER r, INTEGER s }) : on le décode.
     */
    fun sign(data: ByteArray): ByteArray {
        val der = Signature.getInstance("SHA256withECDSA").run { initSign(privateKey()); update(data); sign() }
        val (r, s) = derToRS(der)
        val inner = ByteArrayOutputStream().also { DataOutputStream(it).use { o -> sshMpint(o, r); sshMpint(o, s) } }.toByteArray()
        val bos = ByteArrayOutputStream()
        DataOutputStream(bos).use { o -> sshString(o, ALG.toByteArray()); sshString(o, inner) }
        return bos.toByteArray()
    }

    private fun derToRS(der: ByteArray): Pair<BigInteger, BigInteger> {
        var i = 0
        require(der[i++].toInt() == 0x30) { "DER : SEQUENCE attendue" }
        var len = der[i++].toInt() and 0xff
        if (len and 0x80 != 0) i += (len and 0x7f) // longueur longue (jamais pour P-256, par sûreté)
        fun readInt(): BigInteger {
            require(der[i++].toInt() == 0x02) { "DER : INTEGER attendu" }
            val l = der[i++].toInt() and 0xff
            val v = BigInteger(1, der.copyOfRange(i, i + l)); i += l
            return v
        }
        return readInt() to readInt()
    }

    /** Identité JSch adossée au Keystore (aucune clé privée en mémoire Java). */
    class KeystoreIdentity : Identity {
        override fun setPassphrase(passphrase: ByteArray?): Boolean = true
        override fun getPublicKeyBlob(): ByteArray = publicBlob()
        override fun getSignature(data: ByteArray): ByteArray = sign(data)
        override fun getAlgName(): String = ALG
        override fun getName(): String = ALIAS
        override fun isEncrypted(): Boolean = false
        override fun clear() {}
    }
}
