package com.jbr.purpleops

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.DashPathEffect
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import androidx.core.content.ContextCompat
import androidx.core.content.res.ResourcesCompat

/**
 * Kit de composants « Neon Ops » pour les écrans construits en code
 * (Accueil, Chat…). Même rendu que les layouts XML et que les maquettes :
 * titres Space Grotesk, libellés/IDs JetBrains Mono, cartes ardoise,
 * accent violet en contour, couleurs d'état de soc-display.
 */
class Neon(private val ctx: Context) {

    val display: Typeface = ResourcesCompat.getFont(ctx, R.font.space_grotesk) ?: Typeface.DEFAULT_BOLD
    val mono: Typeface = ResourcesCompat.getFont(ctx, R.font.jetbrains_mono) ?: Typeface.MONOSPACE
    val displayBold: Typeface = Typeface.create(display, 700, false)
    val monoBold: Typeface = Typeface.create(mono, 600, false)

    fun c(id: Int) = ContextCompat.getColor(ctx, id)
    val text get() = c(R.color.text)
    val soft get() = c(R.color.text_soft)
    val muted get() = c(R.color.muted)
    val faint get() = c(R.color.faint)
    val accent get() = c(R.color.accent)
    val accentSoft get() = c(R.color.accent_soft)
    val ok get() = c(R.color.ok)
    val warn get() = c(R.color.warn)
    val crit get() = c(R.color.danger)
    val info get() = c(R.color.info)
    val border get() = c(R.color.border)

    fun dp(v: Int) = (v * ctx.resources.displayMetrics.density).toInt()
    fun dpf(v: Float) = v * ctx.resources.displayMetrics.density

    fun lp(h: Int = ViewGroup.LayoutParams.WRAP_CONTENT) =
        LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, h)
    fun weight(w: Float = 1f) = LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, w)

    // ------------------------------------------------------------ texte

    fun text(s: String, size: Float, color: Int = text, bold: Boolean = false) = TextView(ctx).apply {
        this.text = s; textSize = size; setTextColor(color)
        if (bold) setTypeface(typeface, Typeface.BOLD)
    }

    /** Titre Space Grotesk. */
    fun title(s: String, size: Float = 15f, color: Int = text) = TextView(ctx).apply {
        this.text = s; textSize = size; setTextColor(color); typeface = displayBold
        letterSpacing = 0.02f
    }

    /** Libellé HUD en capitales mono (« POSTURE SOC »). */
    fun kicker(s: String, color: Int = muted) = TextView(ctx).apply {
        this.text = s.uppercase(); textSize = 11f; setTextColor(color); typeface = mono; letterSpacing = 0.12f
    }

    /** Valeur mono (IDs, IP, heures, compteurs). */
    fun monoText(s: String, size: Float = 12f, color: Int = text, bold: Boolean = false) = TextView(ctx).apply {
        this.text = s; textSize = size; setTextColor(color); typeface = if (bold) monoBold else mono
    }

    fun ellipsize(t: TextView) = t.apply { maxLines = 1; ellipsize = TextUtils.TruncateAt.END }

    // ------------------------------------------------------------ structure

    fun row(gravity: Int = Gravity.CENTER_VERTICAL) = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL; this.gravity = gravity; layoutParams = lp()
    }

    fun column() = LinearLayout(ctx).apply { orientation = LinearLayout.VERTICAL; layoutParams = lp() }

    fun spacer() = View(ctx).apply { layoutParams = LinearLayout.LayoutParams(0, 1, 1f) }

    /** Titre de section posé AU-DESSUS d'une carte (comme sur la maquette). */
    fun section(t: String, right: String = "", rightColor: Int = muted) = row().apply {
        setPadding(dp(2), dp(10), dp(2), dp(8))
        addView(title(t).apply { layoutParams = weight() })
        if (right.isNotEmpty()) addView(monoText(right, 11f, rightColor))
    }

    fun card(bg: Int = R.drawable.card, padH: Int = 14, padV: Int = 14) = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundResource(bg)
        setPadding(dp(padH), dp(padV), dp(padH), dp(padV))
        layoutParams = lp().apply { bottomMargin = dp(8) }
    }

    fun tile(bg: Int = R.drawable.tile) = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundResource(bg)
        setPadding(dp(12), dp(10), dp(12), dp(10))
    }

    fun divider() = View(ctx).apply {
        setBackgroundColor(c(R.color.border)); layoutParams = lp(dp(1)).apply { topMargin = dp(10); bottomMargin = dp(10) }
    }

    // ------------------------------------------------------------ état

    fun dot(color: Int, sizeDp: Int = 8, glow: Boolean = true, hollow: Boolean = false) = View(ctx).apply {
        background = GradientDrawable().apply {
            shape = GradientDrawable.OVAL
            if (hollow) { setColor(Color.TRANSPARENT); setStroke(dp(2), color) } else setColor(color)
        }
        if (glow && !hollow) { outlineSpotShadowColor = color; outlineAmbientShadowColor = color; elevation = dpf(3f) }
        layoutParams = LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp)).apply { gravity = Gravity.CENTER_VERTICAL }
    }

    /** Pastille d'état arrondie (contour + voile de la couleur). */
    fun chip(s: String, color: Int, withDot: Boolean = false) = LinearLayout(ctx).apply {
        orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL
        background = GradientDrawable().apply {
            cornerRadius = dpf(999f); setColor(fade(color, 0.10f)); setStroke(dp(1), fade(color, 0.40f))
        }
        setPadding(dp(10), dp(4), dp(10), dp(4))
        layoutParams = LinearLayout.LayoutParams(-2, -2).apply { marginEnd = dp(6) }
        if (withDot) addView(dot(color, 6).apply { (layoutParams as LinearLayout.LayoutParams).marginEnd = dp(6) })
        addView(monoText(s, 11f, lighten(color)))
    }

    /** Badge de niveau carré (12, 10…) contouré. */
    fun levelBadge(level: Int, color: Int, sizeDp: Int = 36) = monoText("$level", 14f, lighten(color), bold = true).apply {
        gravity = Gravity.CENTER
        background = GradientDrawable().apply { cornerRadius = dpf(10f); setColor(fade(color, 0.14f)); setStroke(dp(1), color) }
        layoutParams = LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp))
    }

    fun bar(frac: Double, color: Int, heightDp: Int = 4) = LinearLayout(ctx).apply {
        background = GradientDrawable().apply { cornerRadius = dpf(4f); setColor(c(R.color.border)) }
        val f = frac.coerceIn(0.0, 1.0).toFloat()
        addView(View(ctx).apply {
            background = GradientDrawable().apply { cornerRadius = dpf(4f); setColor(color) }
            layoutParams = LinearLayout.LayoutParams(0, -1, f.coerceAtLeast(0.001f))
        })
        addView(View(ctx).apply { layoutParams = LinearLayout.LayoutParams(0, -1, (1f - f).coerceAtLeast(0.001f)) })
        layoutParams = lp(dp(heightDp))
    }

    // ------------------------------------------------------------ boutons

    enum class Kind { PRIMARY, SECONDARY, GHOST, DANGER, OUTLINE_DANGER }

    fun button(s: String, kind: Kind = Kind.SECONDARY, onClick: () -> Unit) = TextView(ctx).apply {
        text = s; textSize = 13f; setTypeface(typeface, Typeface.BOLD); gravity = Gravity.CENTER
        minHeight = dp(40); setPadding(dp(14), dp(8), dp(14), dp(8))
        when (kind) {
            Kind.PRIMARY -> { setBackgroundResource(R.drawable.btn_accent); setTextColor(c(R.color.bg)) }
            Kind.SECONDARY -> { setBackgroundResource(R.drawable.btn_neon); setTextColor(c(R.color.accent_on)) }
            Kind.GHOST -> { setBackgroundResource(R.drawable.btn_ghost); setTextColor(soft) }
            Kind.DANGER -> { setBackgroundResource(R.drawable.btn_danger); setTextColor(Color.WHITE); minHeight = dp(52) }
            Kind.OUTLINE_DANGER -> { setBackgroundResource(R.drawable.btn_outline_danger); setTextColor(c(R.color.danger_soft)) }
        }
        isClickable = true; isFocusable = true
        setOnClickListener { onClick() }
    }

    /** Tuile-raccourci : icône accent + libellé. */
    fun shortcut(icon: Int, label: String, onClick: () -> Unit) = LinearLayout(ctx).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundResource(R.drawable.btn_tile)
        setPadding(dp(12), dp(12), dp(12), dp(12))
        minimumHeight = dp(64)
        addView(ImageView(ctx).apply {
            setImageResource(icon); imageTintList = ContextCompat.getColorStateList(ctx, R.color.accent_soft)
            layoutParams = LinearLayout.LayoutParams(dp(20), dp(20)).apply { bottomMargin = dp(6) }
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        })
        addView(text(label, 12f, text))
        isClickable = true; isFocusable = true; contentDescription = label
        setOnClickListener { onClick() }
    }

    // ------------------------------------------------------------ couleurs

    fun fade(c: Int, a: Float) = Color.argb((255 * a).toInt(), Color.red(c), Color.green(c), Color.blue(c))

    /** Variante claire d'une couleur d'état, lisible sur fond sombre (texte des pastilles). */
    fun lighten(col: Int): Int = when (col) {
        ok -> c(R.color.ok_soft)
        warn -> c(R.color.warn_soft)
        crit -> c(R.color.danger_soft)
        accent -> c(R.color.accent_soft)
        else -> col
    }
}

/**
 * Jauge circulaire de la carte « Posture SOC » : anneau de fond, arc de la
 * couleur d'état, cercle intérieur en pointillés (HUD). Le texte est posé
 * par-dessus par l'appelant (FrameLayout).
 */
class GaugeView(ctx: Context) : View(ctx) {
    var fraction = 0f
        set(v) { field = v.coerceIn(0f, 1f); invalidate() }
    var color = ContextCompat.getColor(ctx, R.color.accent)
        set(v) { field = v; invalidate() }

    private val d = ctx.resources.displayMetrics.density
    private val track = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE; strokeWidth = 8 * d; color = ContextCompat.getColor(ctx, R.color.border)
    }
    private val arc = Paint(Paint.ANTI_ALIAS_FLAG).apply { style = Paint.Style.STROKE; strokeWidth = 8 * d; strokeCap = Paint.Cap.ROUND }
    private val inner = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        style = Paint.Style.STROKE; strokeWidth = 1 * d
        pathEffect = DashPathEffect(floatArrayOf(2 * d, 4 * d), 0f)
    }
    private val box = RectF()

    override fun onDraw(canvas: Canvas) {
        val s = minOf(width, height).toFloat()
        val pad = 6 * d
        box.set(pad, pad, s - pad, s - pad)
        canvas.drawArc(box, 0f, 360f, false, track)
        arc.color = color
        arc.setShadowLayer(8 * d, 0f, 0f, Color.argb(110, Color.red(color), Color.green(color), Color.blue(color)))
        if (fraction > 0f) canvas.drawArc(box, -90f, 360f * fraction, false, arc)
        inner.color = Color.argb(70, Color.red(color), Color.green(color), Color.blue(color))
        val r = (s / 2f) - 18 * d
        canvas.drawCircle(s / 2f, s / 2f, r, inner)
    }

    init { setLayerType(LAYER_TYPE_SOFTWARE, null) } // setShadowLayer (halo) sur un tracé
}
