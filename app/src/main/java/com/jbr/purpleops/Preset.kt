package com.jbr.purpleops

/** Un bouton = une commande preenregistree. {T} est remplace par la cible saisie. */
data class Preset(
    val label: String,
    val cmd: String,
    val needsTarget: Boolean
)
