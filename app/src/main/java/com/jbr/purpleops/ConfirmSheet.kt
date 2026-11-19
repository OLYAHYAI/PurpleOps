package com.jbr.purpleops

import android.text.InputType
import android.view.LayoutInflater
import android.view.View
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import com.google.android.material.bottomsheet.BottomSheetBehavior
import com.google.android.material.bottomsheet.BottomSheetDialog

/**
 * Feuille de confirmation « Neon Ops » : une seule forme pour toutes les
 * actions sensibles (offensives, arrêts, oublis). Remplace les AlertDialog
 * construites à la main, qui ne suivaient pas le thème.
 *
 * Usage :
 * ```
 * ConfirmSheet(act, "Deauth-test · 5 trames", "Lancer")
 *     .kicker("ACTION OFFENSIVE · AUDITÉE")
 *     .field("bssid", "BSSID de l'AP")
 *     .warn("…")
 *     .show { values -> … }
 * ```
 */
class ConfirmSheet(
    private val act: AppCompatActivity,
    private val title: String,
    private val confirmLabel: String,
) {
    enum class Tone { DANGER, ACCENT }

    private data class Field(val key: String, val label: String, val hint: String, val numeric: Boolean, val initial: String)

    private var kicker: String? = null
    private var message: String? = null
    private var warn: String? = null
    private var tone = Tone.DANGER
    private val fields = mutableListOf<Field>()

    fun kicker(s: String) = apply { kicker = s }
    fun message(s: String) = apply { message = s }
    fun warn(s: String) = apply { warn = s }
    fun tone(t: Tone) = apply { tone = t }
    fun field(key: String, label: String, hint: String = "", numeric: Boolean = false, initial: String = "") =
        apply { fields += Field(key, label, hint, numeric, initial) }

    /** Affiche la feuille ; [onConfirm] reçoit les valeurs saisies (rognées), par clé. */
    fun show(onConfirm: (Map<String, String>) -> Unit) {
        val dialog = BottomSheetDialog(act)
        val v = LayoutInflater.from(act).inflate(R.layout.sheet_confirm, null)
        val d = act.resources.displayMetrics.density
        fun dp(x: Int) = (x * d).toInt()

        v.findViewById<TextView>(R.id.sheetTitle).text = title
        v.findViewById<TextView>(R.id.sheetKicker).apply {
            text = kicker.orEmpty(); visibility = if (kicker == null) View.GONE else View.VISIBLE
        }
        v.findViewById<TextView>(R.id.sheetMessage).apply {
            text = message.orEmpty(); visibility = if (message == null) View.GONE else View.VISIBLE
        }
        v.findViewById<TextView>(R.id.sheetWarn).apply {
            text = warn.orEmpty(); visibility = if (warn == null) View.GONE else View.VISIBLE
        }

        val box = v.findViewById<LinearLayout>(R.id.sheetFields)
        val inputs = LinkedHashMap<String, EditText>()
        for (f in fields) {
            box.addView(TextView(act).apply {
                text = f.label; textSize = 12f
                setTextColor(ContextCompat.getColor(act, R.color.muted))
                setPadding(0, dp(12), 0, dp(6))
            })
            val et = EditText(act).apply {
                setText(f.initial); hint = f.hint; setSingleLine(); textSize = 13f
                typeface = android.graphics.Typeface.MONOSPACE
                setBackgroundResource(R.drawable.bg_input)
                setTextColor(ContextCompat.getColor(act, R.color.text))
                setHintTextColor(ContextCompat.getColor(act, R.color.faint))
                inputType = if (f.numeric) InputType.TYPE_CLASS_NUMBER else InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_NO
            }
            inputs[f.key] = et
            box.addView(et)
        }

        val icon = v.findViewById<android.widget.ImageView>(R.id.sheetIcon)
        val kick = v.findViewById<TextView>(R.id.sheetKicker)
        v.findViewById<TextView>(R.id.sheetConfirm).apply {
            text = confirmLabel
            if (tone == Tone.ACCENT) {
                setBackgroundResource(R.drawable.btn_accent)
                setTextColor(ContextCompat.getColor(act, R.color.bg))
                icon.setBackgroundResource(R.drawable.chip_root)
                icon.imageTintList = ContextCompat.getColorStateList(act, R.color.accent_soft)
                kick.setTextColor(ContextCompat.getColor(act, R.color.accent_soft))
            }
            setOnClickListener {
                val values = inputs.mapValues { it.value.text.toString().trim() }
                dialog.dismiss()
                onConfirm(values)
            }
        }
        v.findViewById<TextView>(R.id.sheetCancel).setOnClickListener { dialog.dismiss() }

        dialog.setContentView(v)
        dialog.behavior.state = BottomSheetBehavior.STATE_EXPANDED
        dialog.behavior.skipCollapsed = true
        dialog.show()
    }
}
