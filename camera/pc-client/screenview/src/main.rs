//! screenview — client PC (Windows) du partage d'écran USB de PurpleOps.
//!
//! Incrément 1 : affiche l'image de l'écran du téléphone (tirage continu) et
//! renvoie un toucher à chaque clic gauche. Mesure la latence aller-retour et
//! les images/s.
//!
//! Usage : screenview [HOTE] [PORT] [JETON]
//!   HOTE  défaut 192.168.42.129 (passerelle du partage de connexion USB)
//!   PORT  défaut 8099
//!   JETON les 32 hex affichés sur le téléphone (sinon variable SCREENVIEW_TOKEN)
//!
//! Protocole « PPSCR1 » : voir crates/screenctl/src/main.rs.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};

const PROTO: &[u8] = b"PPSCR1 ";
const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Hauteur max de la fenêtre PC ; on remet l'image à l'échelle sous ce plafond.
const MAX_WIN_H: usize = 820;

fn main() {
    let mut a = std::env::args().skip(1);
    let host = a.next().unwrap_or_else(|| "192.168.42.129".to_string());
    let port: u16 = a.next().and_then(|p| p.parse().ok()).unwrap_or(8099);
    let token = a
        .next()
        .or_else(|| std::env::var("SCREENVIEW_TOKEN").ok())
        .unwrap_or_default();

    if token.len() != 32 {
        eprintln!("Jeton manquant ou invalide (32 hex attendus).");
        eprintln!("Usage : screenview [HOTE] [PORT] [JETON]   (ou variable SCREENVIEW_TOKEN)");
        std::process::exit(2);
    }

    if let Err(e) = run(&host, port, &token) {
        eprintln!("screenview: {e}");
        std::process::exit(1);
    }
}

fn run(host: &str, port: u16, token: &str) -> io::Result<()> {
    let mut s = TcpStream::connect((host, port))?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;

    // Poignée de main.
    s.write_all(PROTO)?;
    s.write_all(token.as_bytes())?;
    s.write_all(b"\n")?;
    s.flush()?;
    println!("Connecté à {host}:{port}, jeton accepté. Récupération de la première trame…");

    // Première trame pour dimensionner la fenêtre.
    let first = get_frame(&mut s)?;
    let (dw, dh) = (first.w, first.h);
    if dw == 0 || dh == 0 {
        return Err(err("l'appareil n'a renvoyé aucune trame (écran verrouillé ?)"));
    }

    // Facteur de réduction entier pour tenir sous MAX_WIN_H.
    let scale = ((dh + MAX_WIN_H - 1) / MAX_WIN_H).max(1);
    let (ww, wh) = (dw / scale, dh / scale);
    println!("Écran {dw}x{dh}, fenêtre {ww}x{wh} (réduction 1/{scale}).");

    let mut win = Window::new(
        "PurpleOps — écran du téléphone (USB)  [clic = toucher, Échap = quitter]",
        ww,
        wh,
        WindowOptions::default(),
    )
    .map_err(|e| err(&format!("fenêtre : {e}")))?;
    win.limit_update_rate(None); // on cadence nous-mêmes (tirage continu)

    let mut buf = vec![0u32; ww * wh];
    blit(&first, &mut buf, ww, wh, scale);

    let mut frames = 0u32;
    let mut lat_sum = Duration::ZERO;
    let mut since = Instant::now();
    let mut prev_down = false;

    while win.is_open() && !win.is_key_down(Key::Escape) {
        // Clic gauche -> toucher (front montant seulement, pour un tap par clic).
        let down = win.get_mouse_down(MouseButton::Left);
        if down && !prev_down {
            if let Some((mx, my)) = win.get_mouse_pos(MouseMode::Discard) {
                let x = (mx as usize * scale).min(dw.saturating_sub(1)) as u32;
                let y = (my as usize * scale).min(dh.saturating_sub(1)) as u32;
                send_tap(&mut s, x, y)?;
            }
        }
        prev_down = down;

        // Tirage d'une trame + mesure de latence aller-retour.
        let t0 = Instant::now();
        let f = get_frame(&mut s)?;
        lat_sum += t0.elapsed();
        if f.w == dw && f.h == dh {
            blit(&f, &mut buf, ww, wh, scale);
        }

        win.update_with_buffer(&buf, ww, wh).map_err(|e| err(&format!("maj fenêtre : {e}")))?;

        frames += 1;
        if since.elapsed() >= Duration::from_secs(1) {
            let fps = frames as f64 / since.elapsed().as_secs_f64();
            let lat = if frames > 0 { lat_sum.as_secs_f64() * 1000.0 / frames as f64 } else { 0.0 };
            win.set_title(&format!(
                "PurpleOps — {dw}x{dh}  |  {fps:.1} img/s  |  latence {lat:.0} ms  [clic=toucher, Échap=quitter]"
            ));
            println!("{fps:.1} img/s  latence aller-retour {lat:.0} ms");
            frames = 0;
            lat_sum = Duration::ZERO;
            since = Instant::now();
        }
    }
    println!("Fermeture.");
    Ok(())
}

struct Frame {
    w: usize,
    h: usize,
    bpp: usize,
    pixels: Vec<u8>,
}

fn get_frame(s: &mut TcpStream) -> io::Result<Frame> {
    s.write_all(&[0x01])?;
    s.flush()?;
    let w = read_u32(s)? as usize;
    let h = read_u32(s)? as usize;
    let bpp = read_u32(s)? as usize;
    let len = read_u32(s)? as usize;
    if len > MAX_FRAME {
        return Err(err("trame annoncée trop grande"));
    }
    let mut pixels = vec![0u8; len];
    if len > 0 {
        s.read_exact(&mut pixels)?;
    }
    Ok(Frame { w, h, bpp, pixels })
}

fn send_tap(s: &mut TcpStream, x: u32, y: u32) -> io::Result<()> {
    let mut msg = Vec::with_capacity(9);
    msg.push(0x02);
    msg.extend_from_slice(&x.to_be_bytes());
    msg.extend_from_slice(&y.to_be_bytes());
    s.write_all(&msg)?;
    s.flush()?;
    let mut ack = [0u8; 1];
    s.read_exact(&mut ack)?;
    Ok(())
}

/// Recopie une trame RGBA dans le tampon minifb (0x00RRGGBB), avec réduction 1/scale
/// en plus proche voisin.
fn blit(f: &Frame, buf: &mut [u32], ww: usize, wh: usize, scale: usize) {
    if f.bpp != 4 || f.pixels.len() < f.w * f.h * 4 {
        return;
    }
    let row = f.w * 4;
    for wy in 0..wh {
        let sy = wy * scale;
        for wx in 0..ww {
            let sx = wx * scale;
            let i = sy * row + sx * 4;
            let (r, g, b) = (f.pixels[i] as u32, f.pixels[i + 1] as u32, f.pixels[i + 2] as u32);
            buf[wy * ww + wx] = (r << 16) | (g << 8) | b;
        }
    }
}

fn read_u32(s: &mut TcpStream) -> io::Result<u32> {
    let mut b = [0u8; 4];
    s.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.to_string())
}
