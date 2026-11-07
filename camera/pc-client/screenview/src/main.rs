//! screenview — client PC (Windows) du partage d'écran USB de PurpleOps.
//!
//! Affiche l'écran du téléphone et renvoie les gestes de la souris :
//!   clic gauche  = toucher        glisser (bouton gauche) = balayage / défilement
//!   clic droit   = Retour         clic molette            = Accueil
//!   Échap        = quitter
//!
//! Deux modes :
//!   défaut   flux H.264 (encodeur matériel du téléphone via `screenrecord`),
//!            décodé ici par OpenH264 — fluide, quelques Mo/s ;
//!   --raw    images brutes RGBA à la demande (incrément 1, pour comparer).
//!
//! Usage : screenview [JETON] [HOTE] [PORT] [--raw] [--mbps N]  (ordre libre)
//!   JETON  les 32 hex affichés dans PurpleOps ; demandé s'il manque
//!   HOTE   trouvée automatiquement (passerelle de la carte « UsbNcm »)
//!   PORT   défaut 8099 ;  --mbps  débit vidéo demandé (défaut 8)
//!
//! Protocole « PPSCR1 » : voir crates/screenctl/src/main.rs.

use std::io::{self, BufRead, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use minifb::{Key, MouseButton, MouseMode, Window, WindowOptions};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

const PROTO: &[u8] = b"PPSCR1 ";
const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Hauteur max de la fenêtre PC ; l'image est remise à l'échelle sous ce plafond.
const MAX_WIN_H: usize = 960;
/// Au-delà de ce déplacement (pixels téléphone), un clic devient un balayage.
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
    eprintln!("\nAppuie sur Entrée pour fermer.");
    let _ = io::stdin().lock().read_line(&mut String::new());
    std::process::exit(code);
}

/// Image prête à afficher : déjà à la taille de la fenêtre (0x00RRGGBB).
struct Shown {
    buf: Vec<u32>,
}

/// Géométrie téléphone <-> fenêtre, fixée à la première image.
#[derive(Clone, Copy)]
struct Geo {
    dw: usize,
    dh: usize,
    ww: usize,
    wh: usize,
}

impl Geo {
    fn new(dw: usize, dh: usize) -> Self {
        let f = (dh as f32 / MAX_WIN_H as f32).max(1.0);
        Geo { dw, dh, ww: (dw as f32 / f).round() as usize, wh: (dh as f32 / f).round() as usize }
    }
    fn to_dev(self, p: (f32, f32)) -> (u32, u32) {
        let x = (p.0 * self.dw as f32 / self.ww as f32) as usize;
        let y = (p.1 * self.dh as f32 / self.wh as f32) as usize;
        (x.min(self.dw - 1) as u32, y.min(self.dh - 1) as u32)
    }
    /// Réduit une image RGB(A) (stride = largeur * bpp) en tampon fenêtre.
    fn scale(&self, px: &[u8], bpp: usize, out: &mut [u32]) {
        let row = self.dw * bpp;
        for wy in 0..self.wh {
            let sy = wy * self.dh / self.wh;
            for wx in 0..self.ww {
                let i = sy * row + (wx * self.dw / self.ww) * bpp;
                out[wy * self.ww + wx] = (px[i] as u32) << 16 | (px[i + 1] as u32) << 8 | px[i + 2] as u32;
            }
        }
    }
}

enum Cmd {
    Tap(u32, u32),
    Swipe(u32, u32, u32, u32, u32),
    Key(u32),
}

impl Cmd {
    fn encode(&self) -> Vec<u8> {
        let (op, vals): (u8, Vec<u32>) = match *self {
            Cmd::Tap(x, y) => (0x02, vec![x, y]),
            Cmd::Swipe(a, b, c, d, ms) => (0x03, vec![a, b, c, d, ms]),
            Cmd::Key(k) => (0x04, vec![k]),
        };
        let mut m = vec![op];
        for v in vals {
            m.extend_from_slice(&v.to_be_bytes());
        }
        m
    }
}

fn real_main() -> io::Result<()> {
    let (mut token, mut host, mut port, mut raw, mut mbps) = (None, None, 8099u16, false, 8u32);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--raw" {
            raw = true;
        } else if a == "--mbps" {
            mbps = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
        } else if a.len() == 32 && a.chars().all(|c| c.is_ascii_hexdigit()) {
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
        .map_err(|e| err(&format!("connexion à {host}:{port} impossible ({e}) — service activé ? câble branché ?")))?;
    s.set_nodelay(true)?;
    s.write_all(PROTO)?;
    s.write_all(token.as_bytes())?;
    s.write_all(b"\n")?;

    let shown: Arc<Mutex<Option<Shown>>> = Arc::new(Mutex::new(None));
    let stats: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let (geo_tx, geo_rx) = mpsc::channel::<Geo>();

    // Envoi des gestes : en flux, écriture directe (pas d'accusé) ; en brut, via
    // le fil réseau qui intercale les gestes entre deux images.
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    if raw {
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        let (sh, st) = (shown.clone(), stats.clone());
        thread::spawn(move || {
            if let Err(e) = raw_loop(s, cmd_rx, geo_tx, sh, st) {
                eprintln!("Connexion perdue : {e}");
            }
        });
    } else {
        s.write_all(&[0x05])?;
        s.write_all(&(mbps.clamp(1, 40) * 1_000_000).to_be_bytes())?;
        let mut w = s.try_clone()?;
        thread::spawn(move || {
            for c in cmd_rx {
                if w.write_all(&c.encode()).is_err() {
                    break;
                }
            }
        });
        let (sh, st) = (shown.clone(), stats.clone());
        thread::spawn(move || {
            if let Err(e) = h264_loop(s, geo_tx, sh, st) {
                eprintln!("Flux interrompu : {e}");
            }
        });
    }

    println!("Connecté à {host}:{port} ({}). Attente de la première image…", if raw { "brut" } else { "H.264" });
    let geo = geo_rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| err("aucune image reçue en 15 s : jeton refusé, ou écran éteint ?"))?;
    println!("Écran {}x{}, fenêtre {}x{}.", geo.dw, geo.dh, geo.ww, geo.wh);

    let mut win = Window::new("PurpleOps — écran USB", geo.ww, geo.wh, WindowOptions::default())
        .map_err(|e| err(&format!("fenêtre : {e}")))?;
    win.limit_update_rate(Some(Duration::from_millis(8))); // ~120 Hz : souris réactive
    let mut buf = vec![0u32; geo.ww * geo.wh];

    let mut press: Option<((f32, f32), Instant)> = None;
    let (mut prev_r, mut prev_m) = (false, false);
    let mut last_title = String::new();

    while win.is_open() && !win.is_key_down(Key::Escape) {
        let pos = win.get_mouse_pos(MouseMode::Clamp);
        let left = win.get_mouse_down(MouseButton::Left);
        match (left, press, pos) {
            (true, None, Some(p)) => press = Some((p, Instant::now())),
            (false, Some((start, t0)), Some(end)) => {
                let (x1, y1) = geo.to_dev(start);
                let (x2, y2) = geo.to_dev(end);
                let dist = ((x2 as f32 - x1 as f32).powi(2) + (y2 as f32 - y1 as f32).powi(2)).sqrt();
                let _ = cmd_tx.send(if dist < SWIPE_MIN {
                    Cmd::Tap(x1, y1)
                } else {
                    Cmd::Swipe(x1, y1, x2, y2, t0.elapsed().as_millis() as u32)
                });
                press = None;
            }
            (false, Some(_), None) => press = None,
            _ => {}
        }
        let right = win.get_mouse_down(MouseButton::Right);
        if right && !prev_r {
            let _ = cmd_tx.send(Cmd::Key(KEY_BACK));
        }
        prev_r = right;
        let middle = win.get_mouse_down(MouseButton::Middle);
        if middle && !prev_m {
            let _ = cmd_tx.send(Cmd::Key(KEY_HOME));
        }
        prev_m = middle;

        if let Some(f) = shown.lock().unwrap().take() {
            buf = f.buf;
        }
        let st = stats.lock().unwrap().clone();
        if st != last_title && !st.is_empty() {
            win.set_title(&format!("PurpleOps — {st}  [clic droit = Retour, molette = Accueil, Échap = quitter]"));
            last_title = st;
        }
        win.update_with_buffer(&buf, geo.ww, geo.wh).map_err(|e| err(&format!("maj fenêtre : {e}")))?;
    }
    println!("Fermeture.");
    Ok(())
}

/// Mode flux : lit le H.264 Annex-B, découpe en unités NAL, décode, réduit, publie.
///
/// Une NAL est complète quand le code de début suivant arrive. Pour ne pas garder
/// la dernière en otage (écran immobile = plus rien n'arrive), si le câble reste
/// muet 4 ms on considère la fin du tampon comme complète : `screenrecord` écrit
/// chaque image d'un bloc, et sur l'USB les morceaux d'un même bloc se suivent
/// à quelques microsecondes.
fn h264_loop(mut s: TcpStream, geo_tx: Sender<Geo>, shown: Arc<Mutex<Option<Shown>>>, stats: Arc<Mutex<String>>) -> io::Result<()> {
    let mut st = H264State {
        dec: Decoder::new().map_err(|e| err(&format!("décodeur H.264 : {e}")))?,
        geo: None,
        geo_tx,
        shown,
        rgb: Vec::new(),
        frames: 0,
        errors: 0,
        dec_time: Duration::ZERO,
    };
    s.set_read_timeout(Some(Duration::from_millis(4)))?;
    let mut pending: Vec<u8> = Vec::with_capacity(1 << 20);
    let mut chunk = vec![0u8; 256 * 1024];
    let mut bytes = 0usize;
    let mut since = Instant::now();

    loop {
        match s.read(&mut chunk) {
            Ok(0) => return Err(err("le téléphone a fermé le flux")),
            Ok(n) => {
                bytes += n;
                pending.extend_from_slice(&chunk[..n]);
                let starts = start_codes(&pending);
                if starts.len() >= 2 {
                    for w in starts.windows(2) {
                        st.decode(&pending[w[0]..w[1]]);
                    }
                    let last = *starts.last().unwrap();
                    pending.drain(..last);
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                // Silence : la NAL en attente est complète.
                if let Some(&first) = start_codes(&pending).first() {
                    st.decode(&pending[first..]);
                    pending.clear();
                }
            }
            Err(e) => return Err(e),
        }

        if since.elapsed() >= Duration::from_secs(1) {
            let secs = since.elapsed().as_secs_f64();
            let mut line = format!(
                "H.264 {:.0} img/s, {:.1} Mbit/s, décodage {:.1} ms/img",
                st.frames as f64 / secs,
                bytes as f64 * 8.0 / secs / 1e6,
                if st.frames > 0 { st.dec_time.as_secs_f64() * 1000.0 / st.frames as f64 } else { 0.0 }
            );
            if st.errors > 0 {
                line += &format!(", {} erreurs de décodage (essayer --raw)", st.errors);
            }
            println!("{line}");
            *stats.lock().unwrap() = line;
            (st.frames, st.errors, st.dec_time, bytes) = (0, 0, Duration::ZERO, 0);
            since = Instant::now();
        }
    }
}

struct H264State {
    dec: Decoder,
    geo: Option<Geo>,
    geo_tx: Sender<Geo>,
    shown: Arc<Mutex<Option<Shown>>>,
    rgb: Vec<u8>,
    frames: u32,
    errors: u32,
    dec_time: Duration,
}

impl H264State {
    fn decode(&mut self, nal: &[u8]) {
        let t0 = Instant::now();
        match self.dec.decode(nal) {
            Ok(Some(yuv)) => {
                let (w, h) = yuv.dimensions();
                let tx = &self.geo_tx;
                let g = *self.geo.get_or_insert_with(|| {
                    let g = Geo::new(w, h);
                    let _ = tx.send(g);
                    g
                });
                if (w, h) == (g.dw, g.dh) {
                    self.rgb.resize(w * h * 3, 0);
                    yuv.write_rgb8(&mut self.rgb);
                    let mut out = vec![0u32; g.ww * g.wh];
                    g.scale(&self.rgb, 3, &mut out);
                    *self.shown.lock().unwrap() = Some(Shown { buf: out });
                    self.frames += 1;
                }
            }
            Ok(None) => {}
            Err(_) => self.errors += 1,
        }
        self.dec_time += t0.elapsed();
    }
}

/// Positions des codes de début Annex-B (00 00 01, précédé ou non d'un 00).
fn start_codes(b: &[u8]) -> Vec<usize> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i] == 0 && b[i + 1] == 0 && b[i + 2] == 1 {
            v.push(if i > 0 && b[i - 1] == 0 { i - 1 } else { i });
            i += 3;
        } else {
            i += 1;
        }
    }
    v
}

/// Mode brut (incrément 1) : gestes en attente, puis une image, en boucle.
fn raw_loop(mut s: TcpStream, rx: Receiver<Cmd>, geo_tx: Sender<Geo>, shown: Arc<Mutex<Option<Shown>>>, stats: Arc<Mutex<String>>) -> io::Result<()> {
    let mut geo: Option<Geo> = None;
    let (mut frames, mut lat_sum) = (0u32, Duration::ZERO);
    let mut since = Instant::now();
    loop {
        while let Ok(c) = rx.try_recv() {
            s.write_all(&c.encode())?;
            let mut ack = [0u8; 1];
            s.read_exact(&mut ack)?;
        }
        let t0 = Instant::now();
        s.write_all(&[0x01])?;
        let (w, h, bpp, len) = (read_u32(&mut s)? as usize, read_u32(&mut s)? as usize, read_u32(&mut s)? as usize, read_u32(&mut s)? as usize);
        if len > MAX_FRAME {
            return Err(err("image annoncée trop grande"));
        }
        let mut px = vec![0u8; len];
        s.read_exact(&mut px)?;
        lat_sum += t0.elapsed();
        if w == 0 || h == 0 || bpp != 4 || len < w * h * 4 {
            continue;
        }
        let g = *geo.get_or_insert_with(|| {
            let g = Geo::new(w, h);
            let _ = geo_tx.send(g);
            g
        });
        let mut out = vec![0u32; g.ww * g.wh];
        g.scale(&px, 4, &mut out);
        *shown.lock().unwrap() = Some(Shown { buf: out });

        frames += 1;
        if since.elapsed() >= Duration::from_secs(1) {
            let line = format!(
                "brut {:.1} img/s, latence {:.0} ms",
                frames as f64 / since.elapsed().as_secs_f64(),
                lat_sum.as_secs_f64() * 1000.0 / frames as f64
            );
            println!("{line}");
            *stats.lock().unwrap() = line;
            (frames, lat_sum) = (0, Duration::ZERO);
            since = Instant::now();
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
    io::Error::other(msg.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_codes_3_and_4_bytes() {
        let b = [0, 0, 0, 1, 0x67, 9, 0, 0, 1, 0x68, 0, 0, 0, 1, 0x65];
        assert_eq!(start_codes(&b), vec![0, 6, 10]);
    }

    #[test]
    fn geo_maps_corners() {
        let g = Geo::new(1080, 2400);
        assert_eq!(g.wh, MAX_WIN_H);
        assert_eq!(g.to_dev((0.0, 0.0)), (0, 0));
        let (x, y) = g.to_dev((g.ww as f32, g.wh as f32));
        assert_eq!((x, y), (1079, 2399));
    }
}
