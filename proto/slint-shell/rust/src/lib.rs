//! Prototype go/no-go : coquille PurpleOps en Slint sur Android.
//!
//! Architecture testée (cf. claude/purpleops-energie-refonte.md §4) :
//! - `ProtoActivity` (Kotlin) hérite de `NativeActivity` : Slint dessine toute
//!   l'activité, et les API Android (biométrie, Keystore, autres activités)
//!   restent écrites en Kotlin, appelées depuis Rust par JNI sur l'objet
//!   activité lui-même (pas de FindClass depuis un fil natif) ;
//! - le retour Kotlin → Rust passe par une méthode `external` et
//!   `upgrade_in_event_loop` (mise à jour de l'interface sur son propre fil).

use std::sync::Mutex;
use std::time::Instant;

slint::include_modules!();

/// Accès à l'interface depuis les rappels JNI (autre fil que la boucle Slint).
static UI: Mutex<Option<slint::Weak<AppWindow>>> = Mutex::new(None);

fn with_ui(f: impl FnOnce(AppWindow) + Send + 'static) {
    if let Some(w) = UI.lock().ok().and_then(|g| g.clone()) {
        let _ = w.upgrade_in_event_loop(f);
    }
}

/// 200 lignes factices façon journal d'alertes (critère 6).
fn demo_rows() -> slint::ModelRc<slint::SharedString> {
    let rows: Vec<slint::SharedString> = (0..200)
        .map(|i| {
            format!(
                "{:02}:{:02}  agent-{:03}  règle {}  niveau {}",
                (i / 60) % 24,
                i % 60,
                i % 37,
                100300 + (i * 7) % 500,
                3 + (i % 10)
            )
            .into()
        })
        .collect();
    slint::ModelRc::new(slint::VecModel::from(rows))
}

fn build_ui(t0: Instant) -> AppWindow {
    let ui = AppWindow::new().expect("création de l'interface");
    ui.set_rows(demo_rows());
    *UI.lock().unwrap() = Some(ui.as_weak());

    // Démarrage : temps entre l'entrée dans Rust et la première itération de la
    // boucle d'événements (la fenêtre est alors affichée).
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_startup_text(format!("{} ms", t0.elapsed().as_millis()).into());
        }
    });
    ui
}

// ------------------------------------------------------------------ Android
#[cfg(target_os = "android")]
mod android {
    use super::*;
    use jni::objects::{JObject, JString};
    use jni::sys::jboolean;
    use jni::JNIEnv;

    static APP: Mutex<Option<slint::android::AndroidApp>> = Mutex::new(None);

    /// Appelle une méthode d'instance de ProtoActivity (Kotlin).
    fn call_activity(name: &str, sig: &str) -> Result<jni::objects::JValueOwned<'static>, String> {
        let app = APP.lock().unwrap().clone().ok_or("AndroidApp absent")?;
        let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr() as *mut jni::sys::JavaVM) }
            .map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(app.activity_as_ptr() as jni::sys::jobject) };
        let r = env.call_method(&activity, name, sig, &[]).map_err(|e| {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            e.to_string()
        })?;
        // JValueOwned sans référence locale (V / Z / J) : on peut s'affranchir de la durée de vie.
        Ok(match r {
            jni::objects::JValueGen::Long(v) => jni::objects::JValueGen::Long(v),
            jni::objects::JValueGen::Bool(v) => jni::objects::JValueGen::Bool(v),
            _ => jni::objects::JValueGen::Void,
        })
    }

    #[no_mangle]
    fn android_main(app: slint::android::AndroidApp) {
        let t0 = Instant::now();
        *APP.lock().unwrap() = Some(app.clone());
        slint::android::init(app).expect("initialisation Slint Android");

        let ui = build_ui(t0);

        match call_activity("apkSize", "()J") {
            Ok(jni::objects::JValueGen::Long(n)) => {
                ui.set_apk_text(format!("{:.1} Mo", n as f64 / 1_048_576.0).into())
            }
            Ok(_) => ui.set_apk_text("?".into()),
            Err(e) => ui.set_apk_text(format!("erreur JNI : {e}").into()),
        }

        ui.on_bio_sign(|| {
            with_ui(|ui| ui.set_bio_text("Empreinte demandée…".into()));
            if let Err(e) = call_activity("authenticate", "()V") {
                with_ui(move |ui| {
                    ui.set_bio_text(format!("Appel JNI impossible : {e}").into());
                    ui.set_crit_bio(2);
                });
            }
        });

        ui.on_open_web(|| match call_activity("openWeb", "()V") {
            Ok(_) => with_ui(|ui| ui.set_web_text("Ouverte (activité Kotlin)…".into())),
            Err(e) => with_ui(move |ui| {
                ui.set_web_text(format!("Appel JNI impossible : {e}").into());
                ui.set_crit_web(2);
            }),
        });

        ui.run().expect("boucle d'événements");
    }

    /// Kotlin → Rust : résultat de la biométrie + signature Keystore.
    #[no_mangle]
    pub extern "system" fn Java_com_jbr_purpleops_proto_ProtoActivity_nativeOnAuthResult(
        mut env: JNIEnv,
        _this: JObject,
        ok: jboolean,
        detail: JString,
    ) {
        let d: String = env.get_string(&detail).map(|s| s.into()).unwrap_or_default();
        let ok = ok != 0;
        with_ui(move |ui| {
            ui.set_bio_text(d.into());
            ui.set_crit_bio(if ok { 1 } else { 2 });
        });
    }

    /// Kotlin → Rust : retour de l'activité WebView.
    #[no_mangle]
    pub extern "system" fn Java_com_jbr_purpleops_proto_ProtoActivity_nativeOnWebReturned(
        _env: JNIEnv,
        _this: JObject,
    ) {
        with_ui(|ui| {
            ui.set_web_text("Revenu de la WebView : l'interface Slint a repris la main.".into());
            ui.set_crit_web(1);
        });
    }
}

// ------------------------------------------------------------------ hôte
/// Aperçu sur poste (Linux/Windows) : même interface, sans les appels Android.
#[cfg(not(target_os = "android"))]
pub fn run_desktop() {
    let ui = build_ui(Instant::now());
    ui.set_apk_text("(poste)".into());
    ui.on_bio_sign(|| with_ui(|ui| ui.set_bio_text("Biométrie : uniquement sur le téléphone.".into())));
    ui.on_open_web(|| with_ui(|ui| ui.set_web_text("WebView : uniquement sur le téléphone.".into())));
    ui.run().expect("boucle d'événements");
}
