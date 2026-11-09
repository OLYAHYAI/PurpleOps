//! Aperçu hors écran des 4 onglets (poste uniquement), même moteur logiciel que
//! soc-display --snapshot : `cargo run --example snapshot -- <dossier>`.
//! Taille physique du Pixel 7a : 1080 x 2400, facteur d'échelle 2,625.

use slint::platform::software_renderer::{MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};
use std::rc::Rc;

struct Headless(Rc<MinimalSoftwareWindow>);
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let (w, h) = (1080u32, 2400u32);
    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(win.clone()))).unwrap();
    win.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.625 });
    win.set_size(PhysicalSize::new(w, h));

    let ui = proto::AppWindow::new().unwrap();
    ui.set_rows(proto::demo_rows());
    ui.set_apk_text("—".into());
    ui.set_startup_text("—".into());
    ui.show().unwrap();
    for (i, name) in ["accueil", "operations", "energie", "plus"].iter().enumerate() {
        ui.set_page(i as i32);
        let mut buf = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        win.request_redraw();
        win.draw_if_needed(|r| { r.render(&mut buf, w as usize); });
        let mut rgb = Vec::with_capacity(buf.len() * 3);
        for p in &buf { rgb.extend_from_slice(&[p.red, p.green, p.blue]); }
        let path = format!("{out}/proto-{name}.png");
        let f = std::fs::File::create(&path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&rgb).unwrap();
        eprintln!("{path}");
    }
}
