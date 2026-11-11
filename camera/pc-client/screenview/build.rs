//! Icône PurpleOps embarquée dans screenview.exe (ressource Win32, id 1 :
//! l'Explorateur, la barre des tâches et la fenêtre l'utilisent).
//! Source : branding/purpleops-logo.svg → assets/purpleops.ico (16 à 256 px).
fn main() {
    println!("cargo:rerun-if-changed=assets/purpleops.ico");
    println!("cargo:rerun-if-changed=build.rs");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/purpleops.ico")
            .set("ProductName", "PurpleOps")
            .set("FileDescription", "PurpleOps — écran USB du téléphone")
            .set("CompanyName", "PFE SOAR");
        res.compile().expect("ressources Windows (icône PurpleOps)");
    }
}
