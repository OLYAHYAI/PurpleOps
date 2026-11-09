package com.jbr.purpleops

import android.util.Base64
import java.security.MessageDigest
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import javax.net.ssl.HttpsURLConnection
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager

/**
 * TLS par épinglage de clé publique (SPKI SHA-256, format HPKP : base64).
 *
 * Remplace l'ancien « tout accepter ». Les services du homelab (API Wazuh 55000,
 * Indexer 9200) ont des certificats auto-signés dont le SAN ne contient pas
 * l'IP Tailscale : on ne vérifie donc ni l'autorité ni le nom, mais la clé
 * publique exacte du serveur, validée une fois par l'utilisateur (empreinte
 * comparée à `openssl … | openssl dgst -sha256` sur le manager). Un serveur
 * dont la clé change est refusé jusqu'à une nouvelle validation explicite.
 */
object PinnedTls {

    class PinMismatch(val seen: String) : CertificateException("empreinte du certificat inattendue : $seen")

    fun spkiPin(cert: X509Certificate): String =
        Base64.encodeToString(
            MessageDigest.getInstance("SHA-256").digest(cert.publicKey.encoded),
            Base64.NO_WRAP
        )

    private class PinTrustManager(private val pin: String) : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) =
            throw CertificateException("client non pris en charge")

        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            val leaf = chain?.firstOrNull() ?: throw CertificateException("chaîne vide")
            leaf.checkValidity()
            val seen = spkiPin(leaf)
            if (!MessageDigest.isEqual(seen.toByteArray(), pin.toByteArray())) throw PinMismatch(seen)
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    /** Applique l'épinglage à une connexion HTTPS. Le nom n'est pas vérifié : la clé l'est. */
    fun apply(c: HttpsURLConnection, pin: String) {
        val ctx = SSLContext.getInstance("TLS")
        ctx.init(null, arrayOf<TrustManager>(PinTrustManager(pin)), java.security.SecureRandom())
        c.sslSocketFactory = ctx.socketFactory
        c.setHostnameVerifier { _, _ -> true }
    }

    /**
     * Lit la clé publique présentée par host:port SANS lui faire confiance
     * (aucune donnée applicative n'est envoyée). Sert uniquement à afficher
     * l'empreinte à valider. Renvoie (empreinte, sujet) ou lève une exception.
     */
    fun probe(host: String, port: Int): Pair<String, String> {
        var seen: X509Certificate? = null
        val capture = object : X509TrustManager {
            override fun checkClientTrusted(c: Array<out X509Certificate>?, a: String?) =
                throw CertificateException("client non pris en charge")
            override fun checkServerTrusted(c: Array<out X509Certificate>?, a: String?) {
                seen = c?.firstOrNull()
            }
            override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
        }
        val ctx = SSLContext.getInstance("TLS")
        ctx.init(null, arrayOf<TrustManager>(capture), java.security.SecureRandom())
        (ctx.socketFactory.createSocket() as SSLSocket).use { s ->
            s.soTimeout = 8000
            s.connect(java.net.InetSocketAddress(host, port), 8000)
            s.startHandshake()
        }
        val cert = seen ?: throw CertificateException("aucun certificat reçu")
        return spkiPin(cert) to cert.subjectX500Principal.name
    }
}
