package com.jbr.purpleops

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.view.View
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AlertDialog
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch

/**
 * Réglages › Réveil des nœuds (wol-pi).
 *
 * Appairage en deux temps, sans rien faire confiance implicitement :
 *  1. lecture de la clé d'hôte ed25519 du Pi, affichée avec son empreinte à
 *     comparer à `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` (sur le Pi) ;
 *  2. génération de la clé du téléphone (Keystore) et affichage de la ligne
 *     `authorized_keys` à coller dans /home/wol/.ssh/authorized_keys.
 */
class WolPanel(private val act: AppCompatActivity, root: View, private val ui: CoroutineScope) {

    private val status: TextView = root.findViewById(R.id.wolStatus)
    private val keyLine: TextView = root.findViewById(R.id.wolKeyLine)
    private val ctx: Context get() = act.applicationContext

    init {
        root.findViewById<View>(R.id.wolPair).setOnClickListener { pair() }
        root.findViewById<View>(R.id.wolTest).setOnClickListener { test() }
        root.findViewById<View>(R.id.wolForget).setOnClickListener { forget() }
        keyLine.setOnClickListener { copy(keyLine.text.toString()) }
        refresh()
    }

    private fun refresh() {
        status.text = if (WolClient.isPaired(ctx))
            "Appairé · wol-pi ${WolClient.host(ctx)} · clé du téléphone ${WolKey.fingerprint()}"
        else "Non appairé : APPAIRER, puis coller la ligne affichée sur le Pi."
        if (WolClient.isPaired(ctx)) showKeyLine()
    }

    private fun showKeyLine() {
        val ip = WolClient.phoneTailscaleIp() ?: "100.126.112.23"
        keyLine.text = WolKey.authorizedKeysLine(ip)
        keyLine.visibility = View.VISIBLE
    }

    private fun pair() {
        status.text = "Lecture de la clé d'hôte de wol-pi…"
        ui.launch {
            val probe = try { WolClient.probeHostKey(ctx) } catch (e: Exception) {
                status.text = "Échec : ${e.message}"; return@launch
            }
            val (type, b64, fp) = probe
            if (type != "ssh-ed25519") { status.text = "Clé d'hôte inattendue ($type) : refus."; return@launch }
            AlertDialog.Builder(act)
                .setTitle("Clé d'hôte de wol-pi")
                .setMessage("$type\n$fp\n\nCompare avec, sur le Pi :\nssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub\n\nNe fais confiance que si l'empreinte est identique.")
                .setPositiveButton("Faire confiance") { _, _ ->
                    WolClient.pin(ctx, b64)
                    showKeyLine()
                    status.text = "Clé épinglée. Colle la ligne ci-dessous dans /home/wol/.ssh/authorized_keys (sur le Pi), puis TESTER. Toucher la ligne = copier."
                    copy(keyLine.text.toString())
                }
                .setNegativeButton("Refuser") { _, _ -> status.text = "Appairage annulé." }
                .show()
        }
    }

    private fun test() {
        if (!WolClient.isPaired(ctx)) { toast("Appaire d'abord"); return }
        AlertDialog.Builder(act).setTitle("Tester le réveil ?")
            .setMessage("Le test demande le réveil de F4BLOX (nœud SOC). S'il est allumé, wol-pi répond « déjà allumé » sans rien envoyer. La demande est auditée dans Wazuh.")
            .setPositiveButton("Tester") { _, _ ->
                status.text = "Test en cours…"
                ui.launch {
                    val r = WolClient.wake(ctx, "f4blox")
                    WolClient.audit("f4blox", r)
                    status.text = when (r) {
                        "already_up" -> "✓ OK : wol-pi a répondu « déjà allumé » pour F4BLOX (clé acceptée, clé d'hôte vérifiée)."
                        "sent" -> "✓ OK : paquet magique envoyé à F4BLOX (il était éteint)."
                        else -> "Résultat : $r"
                    }
                }
            }
            .setNegativeButton("Annuler", null).show()
    }

    private fun forget() {
        AlertDialog.Builder(act).setTitle("Oublier wol-pi ?")
            .setMessage("Supprime la clé du téléphone et la clé d'hôte épinglée. Pense à retirer la ligne « purpleops-wake » sur le Pi.")
            .setPositiveButton("Oublier") { _, _ -> WolClient.unpair(ctx); keyLine.visibility = View.GONE; refresh() }
            .setNegativeButton("Annuler", null).show()
    }

    private fun copy(s: String) {
        if (s.isBlank()) return
        (act.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager).setPrimaryClip(ClipData.newPlainText("authorized_keys", s))
        toast("Ligne copiée")
    }

    private fun toast(s: String) = Toast.makeText(act, s, Toast.LENGTH_SHORT).show()
}
