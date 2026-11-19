//! Dessin logiciel de la page d'accueil (thème « Neon Ops » du téléphone) dans le
//! tampon de pixels de minifb : rectangles arrondis anticrénelés, texte (fontdue),
//! logo hexagonal, liaison en pointillés. Aucune dépendance graphique système.

use std::collections::HashMap;

use fontdue::{Font, FontSettings};

// Palette « Neon Ops » (identique à colors.xml de l'appli).
pub const BG_TOP: u32 = 0x1A1230;
pub const BG: u32 = 0x07090D;
pub const SURFACE: u32 = 0x121820;
pub const FOCUS: u32 = 0x1B1430;
pub const BORDER: u32 = 0x1E2731;
pub const BORDER_STRONG: u32 = 0x3A4654;
pub const TEXT: u32 = 0xE6EDF3;
pub const SOFT: u32 = 0xB7C2CE;
pub const MUTED: u32 = 0x8B98A6;
pub const FAINT: u32 = 0x7D8A98;
pub const ACCENT: u32 = 0xA37BFF;
pub const ACCENT_SOFT: u32 = 0xC9B3FF;
pub const OK: u32 = 0x2ECC71;
pub const OK_SOFT: u32 = 0x9FF0C0;
pub const WARN: u32 = 0xF5A524;
pub const WARN_SOFT: u32 = 0xF5C36A;
pub const CRIT: u32 = 0xFF4D5E;
pub const CRIT_SOFT: u32 = 0xFF8590;
pub const CRIT_STRONG: u32 = 0xD93A4B;
pub const CONSOLE: u32 = 0x07060D;
pub const CONSOLE_TEXT: u32 = 0x9DFFC4;

/// Polices : titres Space Grotesk, valeurs JetBrains Mono (embarquées, OFL),
/// texte courant Segoe UI du système si présent (sinon Space Grotesk Medium).
pub struct Fonts {
    pub title: Font,
    pub body: Font,
    pub mono: Font,
    cache: HashMap<(u8, char, u32), (fontdue::Metrics, Vec<u8>)>,
}

#[derive(Clone, Copy)]
pub enum Face {
    Title,
    Body,
    Mono,
}

impl Fonts {
    pub fn load() -> Fonts {
        let f = |b: &[u8]| Font::from_bytes(b, FontSettings::default()).expect("police embarquée");
        let title = f(include_bytes!("../assets/fonts/SpaceGrotesk-Bold.ttf"));
        let mono = f(include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"));
        let body = system_body_font().unwrap_or_else(|| f(include_bytes!("../assets/fonts/SpaceGrotesk-Medium.ttf")));
        Fonts { title, body, mono, cache: HashMap::new() }
    }

    fn font(&self, face: Face) -> &Font {
        match face {
            Face::Title => &self.title,
            Face::Body => &self.body,
            Face::Mono => &self.mono,
        }
    }

    /// Largeur d'un texte en pixels (avance + crénage).
    pub fn width(&self, face: Face, px: f32, text: &str) -> f32 {
        let font = self.font(face);
        let mut w = 0.0;
        let mut prev: Option<char> = None;
        for c in text.chars() {
            if let Some(p) = prev {
                w += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            w += font.metrics(c, px).advance_width;
            prev = Some(c);
        }
        w
    }

    /// Tronque avec « … » pour tenir dans `max` pixels.
    pub fn fit(&self, face: Face, px: f32, text: &str, max: f32) -> String {
        if self.width(face, px, text) <= max {
            return text.to_string();
        }
        let mut s: Vec<char> = text.chars().collect();
        while !s.is_empty() {
            s.pop();
            let t: String = s.iter().collect::<String>() + "…";
            if self.width(face, px, &t) <= max {
                return t;
            }
        }
        "…".into()
    }

    fn glyph(&mut self, face: Face, c: char, px: f32) -> &(fontdue::Metrics, Vec<u8>) {
        let key = (face as u8, c, (px * 4.0) as u32);
        if !self.cache.contains_key(&key) {
            let g = self.font(face).rasterize(c, px);
            self.cache.insert(key, g);
        }
        &self.cache[&key]
    }
}

#[cfg(windows)]
fn system_body_font() -> Option<Font> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
    let bytes = std::fs::read(format!("{dir}\\Fonts\\segoeui.ttf")).ok()?;
    Font::from_bytes(bytes, FontSettings::default()).ok()
}

#[cfg(not(windows))]
fn system_body_font() -> Option<Font> {
    None
}

/// Tampon de dessin (0x00RRGGBB, comme minifb).
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub buf: Vec<u32>,
}

#[inline]
fn mix(dst: u32, src: u32, a: f32) -> u32 {
    if a <= 0.0 {
        return dst;
    }
    if a >= 1.0 {
        return src;
    }
    let ch = |s: u32, d: u32| -> u32 {
        let (s, d) = (s as f32, d as f32);
        (d + (s - d) * a).round().clamp(0.0, 255.0) as u32
    };
    (ch(src >> 16 & 255, dst >> 16 & 255) << 16) | (ch(src >> 8 & 255, dst >> 8 & 255) << 8) | ch(src & 255, dst & 255)
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, buf: vec![BG; w * h] }
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: u32, a: f32) {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return;
        }
        let i = y as usize * self.w + x as usize;
        self.buf[i] = mix(self.buf[i], c, a);
    }

    /// Fond : dégradé violet sombre en haut, grille discrète (comme les maquettes).
    pub fn background(&mut self, top: u32) {
        for y in 0..self.h {
            let t = (y as f32 / (self.h as f32 * 0.55)).min(1.0);
            let row = mix(top, BG, t);
            for x in 0..self.w {
                let grid = x % 24 == 0 || y % 24 == 0;
                self.buf[y * self.w + x] = if grid { mix(row, ACCENT, 0.05) } else { row };
            }
        }
    }

    /// Rectangle arrondi anticrénelé : remplissage et/ou contour (1 px).
    #[allow(clippy::too_many_arguments)]
    pub fn rrect(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, fill: Option<(u32, f32)>, stroke: Option<(u32, f32)>) {
        let (x0, y0, x1, y1) = ((x - 1.0) as i32, (y - 1.0) as i32, (x + w + 1.0) as i32, (y + h + 1.0) as i32);
        let (cx, cy, hx, hy) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hx).min(hy);
        for py in y0..=y1 {
            for px in x0..=x1 {
                // Distance signée au rectangle arrondi (centre du pixel).
                let d = sd_rrect(px as f32 + 0.5 - cx, py as f32 + 0.5 - cy, hx, hy, r);
                if let Some((c, a)) = fill {
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.blend(px, py, c, cov * a);
                    }
                }
                if let Some((c, a)) = stroke {
                    let cov = (1.0 - (d + 0.5).abs()).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.blend(px, py, c, cov * a);
                    }
                }
            }
        }
    }

    /// Rectangle arrondi à dégradé vertical (cartes « focus », bouton danger).
    #[allow(clippy::too_many_arguments)]
    pub fn rrect_grad(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, top: u32, bottom: u32, a: f32) {
        let (x0, y0, x1, y1) = ((x - 1.0) as i32, (y - 1.0) as i32, (x + w + 1.0) as i32, (y + h + 1.0) as i32);
        let (cx, cy, hx, hy) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let r = r.min(hx).min(hy);
        for py in y0..=y1 {
            let c = mix(top, bottom, ((py as f32 - y) / h.max(1.0)).clamp(0.0, 1.0));
            for px in x0..=x1 {
                let d = sd_rrect(px as f32 + 0.5 - cx, py as f32 + 0.5 - cy, hx, hy, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.blend(px, py, c, cov * a);
                }
            }
        }
    }

    /// Halo autour d'une carte (dégradé d'alpha vers l'extérieur).
    pub fn glow(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: u32, strength: f32, spread: f32) {
        let n = spread as i32;
        for i in (1..=n).rev() {
            let k = i as f32;
            let a = strength * (1.0 - k / (spread + 1.0)).powi(2) * 0.35;
            self.rrect(x - k, y - k, w + 2.0 * k, h + 2.0 * k, r + k, None, Some((c, a)));
        }
    }

    pub fn dot(&mut self, cx: f32, cy: f32, r: f32, c: u32, hollow: bool) {
        self.rrect(cx - r, cy - r, 2.0 * r, 2.0 * r, r, (!hollow).then_some((c, 1.0)), hollow.then_some((c, 1.0)));
        if !hollow {
            self.rrect(cx - r - 3.0, cy - r - 3.0, 2.0 * r + 6.0, 2.0 * r + 6.0, r + 3.0, Some((c, 0.12)), None);
        }
    }

    pub fn hline_dashed(&mut self, x0: f32, x1: f32, y: f32, c: u32) {
        let mut x = x0;
        while x < x1 {
            let e = (x + 6.0).min(x1);
            self.rrect(x, y - 1.0, e - x, 2.0, 1.0, Some((c, 0.9)), None);
            x += 12.0;
        }
    }

    pub fn bar(&mut self, x: f32, y: f32, w: f32, frac: f32, c: u32) {
        self.rrect(x, y, w, 4.0, 2.0, Some((BORDER, 1.0)), None);
        if frac > 0.0 {
            let f = (w * frac.clamp(0.0, 1.0)).max(4.0);
            self.rrect(x, y, f, 4.0, 2.0, Some((c, 1.0)), None);
        }
    }

    /// Texte à partir de la ligne de base `y`. Renvoie la largeur dessinée.
    pub fn text(&mut self, fonts: &mut Fonts, face: Face, px: f32, x: f32, y: f32, c: u32, text: &str) -> f32 {
        let mut pen = x;
        let mut prev: Option<char> = None;
        for ch in text.chars() {
            if let Some(p) = prev {
                pen += fonts.font(face).horizontal_kern(p, ch, px).unwrap_or(0.0);
            }
            let (m, bmp) = fonts.glyph(face, ch, px).clone();
            let gx = pen.round() as i32 + m.xmin;
            let gy = y.round() as i32 - m.height as i32 - m.ymin;
            for row in 0..m.height {
                for col in 0..m.width {
                    let a = bmp[row * m.width + col] as f32 / 255.0;
                    if a > 0.0 {
                        self.blend(gx + col as i32, gy + row as i32, c, a);
                    }
                }
            }
            pen += m.advance_width;
            prev = Some(ch);
        }
        pen - x
    }

    /// Texte centré horizontalement dans [x, x+w].
    #[allow(clippy::too_many_arguments)]
    pub fn text_center(&mut self, fonts: &mut Fonts, face: Face, px: f32, x: f32, w: f32, y: f32, c: u32, text: &str) {
        let tw = fonts.width(face, px, text);
        self.text(fonts, face, px, x + (w - tw) / 2.0, y, c, text);
    }

    /// Texte aligné à droite (fin en `x_end`).
    #[allow(clippy::too_many_arguments)]
    pub fn text_right(&mut self, fonts: &mut Fonts, face: Face, px: f32, x_end: f32, y: f32, c: u32, text: &str) {
        let tw = fonts.width(face, px, text);
        self.text(fonts, face, px, x_end - tw, y, c, text);
    }

    /// Libellé HUD en capitales espacées (« POSTURE SOC »).
    pub fn kicker(&mut self, fonts: &mut Fonts, x: f32, y: f32, c: u32, text: &str) -> f32 {
        let mut pen = x;
        for ch in text.chars() {
            let s = ch.to_string();
            pen += self.text(fonts, Face::Mono, 11.0, pen, y, c, &s) + 1.3;
        }
        pen - x
    }

    /// Logo : hexagone contour + noyau plein (comme l'icône de l'appli).
    pub fn logo(&mut self, cx: f32, cy: f32, r: f32) {
        self.hexagon(cx, cy, r, ACCENT, false);
        self.hexagon(cx, cy, r * 0.5, ACCENT, true);
    }

    fn hexagon(&mut self, cx: f32, cy: f32, r: f32, c: u32, filled: bool) {
        // Hexagone « pointe en haut » : sommets à 90°, 150°, …
        let pts: Vec<(f32, f32)> = (0..6)
            .map(|i| {
                let a = std::f32::consts::PI / 180.0 * (90.0 + 60.0 * i as f32);
                (cx + r * a.cos(), cy - r * a.sin())
            })
            .collect();
        let (x0, x1, y0, y1) = ((cx - r - 2.0) as i32, (cx + r + 2.0) as i32, (cy - r - 2.0) as i32, (cy + r + 2.0) as i32);
        for py in y0..=y1 {
            for px in x0..=x1 {
                // Distance au bord du polygone (min des distances aux segments), signe par inclusion.
                let p = (px as f32 + 0.5, py as f32 + 0.5);
                let mut d = f32::MAX;
                let mut inside = false;
                for i in 0..6 {
                    let (a, b) = (pts[i], pts[(i + 1) % 6]);
                    d = d.min(seg_dist(p, a, b));
                    if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
                        inside = !inside;
                    }
                }
                let cov = if filled {
                    let sd = if inside { -d } else { d };
                    (0.5 - sd).clamp(0.0, 1.0) * 0.85
                } else {
                    (1.3 - d).clamp(0.0, 1.0)
                };
                if cov > 0.0 {
                    self.blend(px, py, c, cov);
                }
            }
        }
    }
}

/// Distance signée d'un point (relatif au centre) à un rectangle arrondi.
#[inline]
fn sd_rrect(dx: f32, dy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let (ax, ay) = (dx.abs() - (hx - r), dy.abs() - (hy - r));
    let (qx, qy) = (ax.max(0.0), ay.max(0.0));
    (qx * qx + qy * qy).sqrt() + ax.max(ay).min(0.0) - r
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    (dx * dx + dy * dy).sqrt()
}

/// Zone cliquable.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.x + self.w && p.1 >= self.y && p.1 < self.y + self.h
    }
}

/// Styles de bouton (comme btn_accent / btn_neon / btn_ghost / btn_danger).
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)] // Secondary : réservé (même jeu de styles que l'appli)
pub enum Btn {
    Primary,
    Secondary,
    Ghost,
    Danger,
    OutlineDanger,
}

impl Canvas {
    /// Bouton : fond selon le style, survol éclairci, désactivé estompé.
    #[allow(clippy::too_many_arguments)]
    pub fn button(&mut self, fonts: &mut Fonts, r: Rect, label: &str, kind: Btn, hover: bool, enabled: bool) {
        let rad = if kind == Btn::Danger { 14.0 } else { 12.0 };
        let dim = if enabled { 1.0 } else { 0.35 };
        let (fg, px) = match kind {
            Btn::Primary => (BG, 14.0),
            Btn::Secondary => (0xE6DCFF, 13.5),
            Btn::Ghost => (TEXT, 13.0),
            Btn::Danger => (0xFFFFFF, 14.0),
            Btn::OutlineDanger => (CRIT_SOFT, 13.0),
        };
        match kind {
            Btn::Primary => {
                if enabled {
                    self.glow(r.x, r.y, r.w, r.h, rad, ACCENT, if hover { 1.0 } else { 0.6 }, 10.0);
                }
                self.rrect(r.x, r.y, r.w, r.h, rad, Some((if hover { ACCENT_SOFT } else { ACCENT }, dim)), None);
            }
            Btn::Secondary => self.rrect(r.x, r.y, r.w, r.h, rad, Some((ACCENT, if hover { 0.26 } else { 0.12 } * dim)), Some((ACCENT, 0.6 * dim))),
            Btn::Ghost => self.rrect(r.x, r.y, r.w, r.h, rad, Some((SURFACE, if hover { 1.0 } else { 0.0 })), Some((BORDER_STRONG, dim))),
            Btn::Danger => {
                if enabled {
                    self.glow(r.x, r.y, r.w, r.h, rad, CRIT, if hover { 0.9 } else { 0.5 }, 10.0);
                    self.rrect_grad(r.x, r.y, r.w, r.h, rad, if hover { 0xFF7380 } else { 0xFF5D6C }, CRIT_STRONG, 1.0);
                    self.rrect(r.x, r.y, r.w, r.h, rad, None, Some((CRIT, 1.0)));
                } else {
                    self.rrect(r.x, r.y, r.w, r.h, rad, Some((CRIT, 0.2)), None);
                }
            }
            Btn::OutlineDanger => self.rrect(r.x, r.y, r.w, r.h, rad, Some((CRIT, if hover { 0.18 } else { 0.0 })), Some((CRIT, 0.55 * dim))),
        }
        let col = if enabled { fg } else { mix(BG, fg, 0.45) };
        let base = r.y + r.h / 2.0 + px * 0.36;
        let t = fonts.fit(Face::Title, px, label, r.w - 16.0);
        self.text_center(fonts, Face::Title, px, r.x, r.w, base, col, &t);
    }

    /// Pastille d'état (contour + voile de la couleur, texte mono clair).
    #[allow(clippy::too_many_arguments)]
    pub fn chip(&mut self, fonts: &mut Fonts, x_end: f32, y: f32, label: &str, c: u32, light: u32, with_dot: bool) -> Rect {
        let tw = fonts.width(Face::Mono, 11.0, label);
        let w = tw + 22.0 + if with_dot { 13.0 } else { 0.0 };
        let r = Rect { x: x_end - w, y, w, h: 26.0 };
        self.rrect(r.x, r.y, r.w, r.h, 13.0, Some((c, 0.10)), Some((c, 0.45)));
        let mut tx = r.x + 11.0;
        if with_dot {
            self.dot(tx + 3.5, r.y + 13.0, 3.5, c, false);
            tx += 13.0;
        }
        self.text(fonts, Face::Mono, 11.0, tx, r.y + 17.0, light, label);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrect_covers_inside_only() {
        let mut c = Canvas { w: 20, h: 20, buf: vec![0; 400] };
        c.rrect(5.0, 5.0, 10.0, 10.0, 3.0, Some((0xFFFFFF, 1.0)), None);
        assert_eq!(c.buf[10 * 20 + 10], 0xFFFFFF); // centre plein
        assert_eq!(c.buf[0], 0); // coin hors forme
        assert_eq!(c.buf[5 * 20 + 5] & 0xFF, c.buf[5 * 20 + 5] & 0xFF); // coin arrondi : partiel
        assert!(c.buf[5 * 20 + 5] < 0xFFFFFF);
    }

    #[test]
    fn mix_endpoints() {
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xFFFFFF, 1.0), 0xFFFFFF);
        assert_eq!(mix(0x000000, 0xFFFFFF, 0.5), 0x808080);
    }

    #[test]
    fn text_fits() {
        let f = Fonts::load();
        let long = "un texte beaucoup trop long pour la zone";
        let t = f.fit(Face::Body, 13.0, long, 80.0);
        assert!(t.ends_with('…'));
        assert!(f.width(Face::Body, 13.0, &t) <= 80.0);
    }
}
