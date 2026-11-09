package com.jbr.purpleops.proto

import android.app.Activity
import android.os.Bundle
import android.webkit.WebView

/**
 * Activité Kotlin classique ouverte depuis l'interface Slint (critère 4) :
 * c'est le chemin prévu pour l'interface Nikon (WebView de p900ctl), qui ne
 * peut pas vivre dans la surface Slint. Retour = touche Retour.
 */
class WebActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val web = WebView(this)
        web.settings.javaScriptEnabled = false
        web.loadDataWithBaseURL(
            null,
            """<html><body style="background:#0a0e13;color:#e6edf3;font-family:sans-serif;padding:24px">
               <h2 style="color:#a37bff">WebView Kotlin</h2>
               <p>Activité Android classique lancée depuis Rust/Slint.</p>
               <p>Touche <b>Retour</b> : l'interface Slint doit reprendre la main et passer le critère 4 à OK.</p>
               </body></html>""",
            "text/html", "utf-8", null
        )
        setContentView(web)
    }
}
