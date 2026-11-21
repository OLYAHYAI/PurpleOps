//! Barre de commandes de la fenêtre de session, SOUS l'image du téléphone.
//!
//! Elle reste utilisable quand l'image est figée ou que le téléphone est noir
//! (par exemple pendant que l'IA pilote l'écran via le MCP) : ses clics ne sont
//! jamais envoyés au téléphone.
//! - Accueil : ferme la session (comme F12) ;
//! - Couper l'IA : prise de commande MCP refusée (double clic pour confirmer) ;
//! - Récupérer l'écran : le téléphone coupe la session, rétablit veille et
//!   luminosité, réveille l'écran ; retour à l'accueil.

use crate::ui::{Btn, Canvas, Fonts, Rect, ACCENT, BORDER};

pub const BAR_H: usize = 56;
const BG: u32 = 0x0C0E16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BarAct {
    Home,
    IaOff,
    Rescue,
}

/// État affiché par la barre.
#[derive(Clone, PartialEq, Default)]
pub struct BarState {
    /// Premier clic sur « Couper l'IA » : le suivant confirme.
    pub confirm_ia: bool,
    /// Commande en cours (« Récupération… »).
    pub busy: Option<BarAct>,
    /// Retour de la dernière coupure de l'IA (« IA coupée »…).
    pub ia_note: Option<String>,
    pub hover: Option<BarAct>,
}

/// Zones des trois boutons pour une fenêtre de largeur `ww`.
pub fn layout(ww: usize) -> [(Rect, BarAct); 3] {
    let (pad, gap, h) = (10.0, 8.0, 38.0);
    let w = ww as f32 - 2.0 * pad - 2.0 * gap;
    let y = (BAR_H as f32 - h) / 2.0 + 1.0;
    let w1 = (w * 0.26).floor();
    let w2 = (w * 0.32).floor();
    let w3 = w - w1 - w2;
    [
        (Rect { x: pad, y, w: w1, h }, BarAct::Home),
        (Rect { x: pad + w1 + gap, y, w: w2, h }, BarAct::IaOff),
        (Rect { x: pad + w1 + w2 + 2.0 * gap, y, w: w3, h }, BarAct::Rescue),
    ]
}

/// Bouton sous le pointeur (coordonnées relatives à la barre).
pub fn hit(ww: usize, p: (f32, f32)) -> Option<BarAct> {
    layout(ww).iter().find(|(r, _)| r.contains(p)).map(|(_, a)| *a)
}

/// Dessine la barre (ww × BAR_H pixels).
pub fn draw(fonts: &mut Fonts, ww: usize, st: &BarState) -> Vec<u32> {
    let mut c = Canvas::new(ww.max(1), BAR_H);
    c.buf.iter_mut().for_each(|p| *p = BG);
    // Liseré supérieur violet (comme la barre flottante de l'appli).
    c.rrect(0.0, 0.0, ww as f32, 1.0, 0.0, Some((ACCENT, 0.35)), None);
    c.rrect(0.0, 1.0, ww as f32, 1.0, 0.0, Some((BORDER, 1.0)), None);
    let busy = st.busy.is_some();
    for (r, a) in layout(ww) {
        let hover = st.hover == Some(a);
        let (label, kind, enabled) = match a {
            BarAct::Home => ("Accueil".to_string(), Btn::Ghost, !busy),
            BarAct::IaOff => {
                if st.busy == Some(BarAct::IaOff) {
                    ("Coupure…".into(), Btn::OutlineDanger, false)
                } else if st.confirm_ia {
                    ("Confirmer ?".into(), Btn::Danger, true)
                } else {
                    (st.ia_note.clone().unwrap_or_else(|| "Couper l'IA".into()), Btn::OutlineDanger, !busy)
                }
            }
            BarAct::Rescue => {
                if st.busy == Some(BarAct::Rescue) {
                    ("Récupération…".into(), Btn::Danger, false)
                } else {
                    ("Récupérer l'écran".into(), Btn::Danger, !busy)
                }
            }
        };
        c.button(fonts, r, &label, kind, hover && enabled, enabled);
    }
    c.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits_and_hits() {
        for ww in [320usize, 430, 900] {
            let l = layout(ww);
            assert!(l[2].0.x + l[2].0.w <= ww as f32 - 9.0);
            assert_eq!(hit(ww, (l[0].0.x + 2.0, 28.0)), Some(BarAct::Home));
            assert_eq!(hit(ww, (l[2].0.x + l[2].0.w - 2.0, 28.0)), Some(BarAct::Rescue));
            assert_eq!(hit(ww, (2.0, 28.0)), None);
        }
    }

    /// Aperçu PNG (`cargo test -- --ignored bar_preview`) : barre sous une image
    /// de téléphone noire, dans ses trois états.
    #[test]
    #[ignore]
    fn bar_preview() {
        let mut f = Fonts::load();
        let ww = 430;
        let states = [
            BarState::default(),
            BarState { confirm_ia: true, hover: Some(BarAct::IaOff), ..Default::default() },
            BarState { busy: Some(BarAct::Rescue), ..Default::default() },
        ];
        let ph = 120;
        let mut out = format!("P6\n{} {}\n255\n", ww, (ph + BAR_H) * states.len()).into_bytes();
        for st in &states {
            for _ in 0..ww * ph {
                out.extend_from_slice(&[4, 4, 6]); // image figée / écran noir
            }
            for p in draw(&mut f, ww, st) {
                out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
            }
        }
        std::fs::write("/tmp/screenview-barre.ppm", out).unwrap();
    }

    #[test]
    fn draw_size() {
        let mut f = Fonts::load();
        assert_eq!(draw(&mut f, 430, &BarState::default()).len(), 430 * BAR_H);
    }
}
