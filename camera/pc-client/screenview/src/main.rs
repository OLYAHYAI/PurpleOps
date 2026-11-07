//! screenview — client PC (Windows) du partage d'écran USB de PurpleOps.
//!
//! Affiche l'écran du téléphone et renvoie les gestes de la souris :
//!   clic gauche  = toucher        glisser (bouton gauche) = balayage / défilement
//!   clic droit   = Retour         clic molette            = Accueil
//!   Échap        = quitter
//!
//! Le réseau tourne dans son propre fil : la fenêtre reste fluide et aucun clic
//! n'est perdu pendant le téléchargement d'une image.
//!
//! Usage : screenview [JETON] [HOTE] [PORT]   (dans n'importe quel ordre)
//!   JETON  les 32 hex affichés sur le téléphone ; demandé s'il manque
//!   HOTE   adresse du téléphone ; trouvée automatiquement (passerelle de la
//!          carte « UsbNcm ») si absente
//!   PORT   défaut 8099
//!
//! Protocole « PPSCR1 » : voir crates/screenctl/src/main.rs.

use std::io::{self, BufRead, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};

const PROTO: &[u8] = b"PPSCR1 ";
const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Hauteur max de la fenêtre PC ; on remet l'image à l'échelle sous ce plafond.
const MAX_WIN_H: usize = 900;
/// Au-delà de ce déplacement (pixels écran téléphone), un clic devient un balayage.
const SWIPE_MIN: f32 = 30.0;

const KEY_HOME: u32 = 3;
const KEY_BACK: u32 = 4;

fn main() {
    let code = match real_main() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("\nscreenview : {e}");
            1
        }
    };
    // Lancé par double-clic, la console se fermerait avant qu'on lise le message.
    eprintln!("\nAppuie sur Entrée pour fermer.");
    let _ = io::stdin().lock().read_line(&mut String::new());
    std::process::exit(code);
}

fn real_main() -> io::Result<()> {
    let mut token = None;
    let mut host = None;
    let mut port = 8099u16;
    for a in std::env::args().skip(1) {
        if a.len() == 32 && a.chars().all(|c| c.is_ascii_hexdigit()) {
            token = Some(a.to_ascii_lowercase());
        } else if let Ok(p) = a.parse::<u16>() {
            port = p;
        } else {
            host = Some(a);
        }
    }

    let host = match host {
        Some(h) => h,
        None => match find_phone() {
            Some(h) => {
                println!("Téléphone trouvé automatiquement : {h}");
                h
            }
            None => prompt("Adresse du téléphone (affichée dans PurpleOps) : ")?,
        },
    };
    let token = match token {
        Some(t) => t,
        None => prompt("Jeton affiché dans PurpleOps (32 caractères) : ")?.to_ascii_lowercase(),
    };
    if token.len() != 32 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(err("jeton invalide : 32 caractères 0-9 a-f attendus"));
    }

    let mut s = TcpStream::connect((host.as_str(), port))
        .map_err(|e| err(&format!("connexion à {host}:{port} impossible ({e}) — service démarré ? câble branché ?")))?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.write_all(PROTO)?;
    s.write_all(token.as_bytes())?;
    s.write_all(b"\n")?;
    s.flush()?;

    let first = get_frame(&mut s).map_err(|e| err(&format!("pas de réponse : jeton refusé ? ({e})")))?;
    let (dw, dh) = (first.w, first.h);
    if dw == 0 || dh == 0 {
        return Err(err("le téléphone n'a renvoyé aucune image (écran verrouillé ?)"));
    }
    println!("Connecté à {host}:{port}. Écran {dw}x{dh}.");

    let scale = dh.div_ceil(MAX_WIN_H).max(1);
    let (ww, wh) = (dw / scale, dh / scale);

    // État partagé : dernière image reçue + statistiques.
    let latest: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
    let stats: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let (tx, rx) = mpsc::channel::<Cmd>();
    {
        let latest = latest.clone();
        let stats = stats.clone();
        thread::spawn(move || {
            if let Err(e) = net_loop(s, rx, latest, stats) {
                eprintln!("Connexion perdue : {e}");
            }
        });
    }

    let mut win = Window::new(
        "PurpleOps — écran USB",
        ww,
        wh,
        WindowOptions::default(),
    )
    .map_err(|e| err(&format!("fenêtre : {e}")))?;
    win.limit_update_rate(Some(Duration::from_millis(16))); // ~60 Hz pour la souris

    let mut buf = vec![0u32; ww * wh];
    blit(&first, &mut buf, ww, wh, scale);

    let to_dev = |p: (f32, f32)| -> (u32, u32) {
        let x = ((p.0 * scale as f32) as usize).min(dw - 1) as u32;
        let y = ((p.1 * scale as f32) as usize).min(dh - 1) as u32;
        (x, y)
    };

    let mut press: Option<((f32, f32), Instant)> = None;
    let (mut prev_r, mut prev_m) = (false, false);
    let mut last_title = String::new();

    while win.is_open() && !win.is_key_down(Key::Escape) {
        let pos = win.get_mouse_pos(MouseMode::Clamp);

        // Gauche : appui puis relâchement -> toucher, ou balayage si on a bougé.
        let left = win.get_mouse_down(MouseButton::Left);
        match (left, press, pos) {
            (true, None, Some(p)) => press = Some((p, Instant::now())),
            (false, Some((start, t0)), Some(end)) => {
                let (x1, y1) = to_dev(start);
                let (x2, y2) = to_dev(end);
                let dist = ((x2 as f32 - x1 as f32).powi(2) + (y2 as f32 - y1 as f32).powi(2)).sqrt();
                let cmd = if dist < SWIPE_MIN {
                    Cmd::Tap(x1, y1)
                } else {
                    Cmd::Swipe(x1, y1, x2, y2, t0.elapsed().as_millis() as u32)
                };
                let _ = tx.send(cmd);
                press = None;
            }
            (false, Some(_), None) => press = None,
            _ => {}
        }

        let right = win.get_mouse_down(MouseButton::Right);
        if right && !prev_r {
            let _ = tx.send(Cmd::Key(KEY_BACK));
        }
        prev_r = right;
        let middle = win.get_mouse_down(MouseButton::Middle);
        if middle && !prev_m {
            let _ = tx.send(Cmd::Key(KEY_HOME));
        }
        prev_m = middle;

        if let Some(f) = latest.lock().unwrap().take() {
            if f.w == dw && f.h == dh {
                blit(&f, &mut buf, ww, wh, scale);
            }
        }
        let st = stats.lock().unwrap().clone();
        if st != last_title && !st.is_empty() {
            win.set_title(&format!("PurpleOps — {st}  [clic droit = Retour, molette = Accueil, Échap = quitter]"));
            last_title = st;
        }
        win.update_with_buffer(&buf, ww, wh).map_err(|e| err(&format!("maj fenêtre : {e}")))?;
    }
    println!("Fermeture.");
    Ok(())
}

enum Cmd {
    Tap(u32, u32),
    Swipe(u32, u32, u32, u32, u32),
    Key(u32),
}

/// Fil réseau : envoie les gestes en attente, puis tire une image, en boucle.
fn net_loop(mut s: TcpStream, rx: Receiver<Cmd>, latest: Arc<Mutex<Option<Frame>>>, stats: Arc<Mutex<String>>) -> io::Result<()> {
    let mut frames = 0u32;
    let mut lat_sum = Duration::ZERO;
    let mut since = Instant::now();
    loop {
        while let Ok(c) = rx.try_recv() {
            let t0 = Instant::now();
            let what = send_cmd(&mut s, c)?;
            println!("{what} ({} ms)", t0.elapsed().as_millis());
        }
        let t0 = Instant::now();
        let f = get_frame(&mut s)?;
        lat_sum += t0.elapsed();
        *latest.lock().unwrap() = Some(f);

        frames += 1;
        if since.elapsed() >= Duration::from_secs(1) {
            let fps = frames as f64 / since.elapsed().as_secs_f64();
            let lat = lat_sum.as_secs_f64() * 1000.0 / frames as f64;
            let line = format!("{fps:.1} img/s, latence {lat:.0} ms");
            println!("{line}");
            *stats.lock().unwrap() = line;
            frames = 0;
            lat_sum = Duration::ZERO;
            since = Instant::now();
        }
    }
}

fn send_cmd(s: &mut TcpStream, c: Cmd) -> io::Result<String> {
    let (op, vals, what): (u8, Vec<u32>, String) = match c {
        Cmd::Tap(x, y) => (0x02, vec![x, y], format!("toucher {x},{y}")),
        Cmd::Swipe(x1, y1, x2, y2, ms) => (0x03, vec![x1, y1, x2, y2, ms], format!("balayage {x1},{y1} -> {x2},{y2}")),
        Cmd::Key(k) => (0x04, vec![k], format!("touche {}", if k == KEY_BACK { "Retour" } else { "Accueil" })),
    };
    let mut msg = vec![op];
    for v in vals {
        msg.extend_from_slice(&v.to_be_bytes());
    }
    s.write_all(&msg)?;
    s.flush()?;
    let mut ack = [0u8; 1];
    s.read_exact(&mut ack)?;
    Ok(what)
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
        return Err(err("image annoncée trop grande"));
    }
    let mut pixels = vec![0u8; len];
    if len > 0 {
        s.read_exact(&mut pixels)?;
    }
    Ok(Frame { w, h, bpp, pixels })
}

/// Recopie une image RGBA dans le tampon minifb (0x00RRGGBB), réduite 1/scale.
fn blit(f: &Frame, buf: &mut [u32], ww: usize, wh: usize, scale: usize) {
    if f.bpp != 4 || f.pixels.len() < f.w * f.h * 4 {
        return;
    }
    let row = f.w * 4;
    for wy in 0..wh {
        let sy = wy * scale;
        for wx in 0..ww {
            let i = sy * row + wx * scale * 4;
            let (r, g, b) = (f.pixels[i] as u32, f.pixels[i + 1] as u32, f.pixels[i + 2] as u32);
            buf[wy * ww + wx] = (r << 16) | (g << 8) | b;
        }
    }
}

/// Adresse du téléphone = passerelle que son DHCP a donnée à la carte UsbNcm.
fn find_phone() -> Option<String> {
    let ps = "(Get-NetIPConfiguration | Where-Object { $_.InterfaceDescription -like '*NCM*' -or \
              $_.InterfaceDescription -like '*NDIS*' } | Select-Object -First 1).IPv4DefaultGateway.NextHop";
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", ps])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    s.parse::<std::net::Ipv4Addr>().ok().map(|ip| ip.to_string())
}

fn prompt(q: &str) -> io::Result<String> {
    print!("{q}");
    io::stdout().flush()?;
    let mut l = String::new();
    io::stdin().lock().read_line(&mut l)?;
    Ok(l.trim().to_string())
}

fn read_u32(s: &mut TcpStream) -> io::Result<u32> {
    let mut b = [0u8; 4];
    s.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}

fn err(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.to_string())
}

