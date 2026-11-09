fn main() {
    // Style « material-dark » pour les widgets standard (LineEdit, ListView) :
    // cohérent avec Android et avec le fond sombre du thème D4.
    let config = slint_build::CompilerConfiguration::new().with_style("material-dark".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("compilation de l'interface Slint");
}
