//! screenview — client PC (Windows) de l'écran USB de PurpleOps.
//!
//! Double-clic : une seule fenêtre, celle du téléphone (pas de console ; l'état
//! s'affiche dans son titre). Le téléphone est trouvé seul (passerelle de la carte
//! « UsbNcm ») ; ce PC s'identifie par une clé gardée dans
//! %APPDATA%\PurpleOps\screenview.key. Première fois : « APPAIRER UN PC » dans
//! PurpleOps, la connexion se fait d'elle-même.
//!
//! Souris : gauche = doigt · molette = défiler · droit = Retour · milieu = Accueil
//! Clavier : texte (accents compris), Entrée, Effacer, flèches, Ctrl+A/C/V/X/Z ·
//!           Échap = Retour
//! Presse-papiers synchronisé dans les deux sens (texte).
//!
//! Options : [HOTE] [PORT] [--mbps N] · journal : %APPDATA%\PurpleOps\screenview.log
//! Protocole « PPSCR3 » : voir crates/screenctl/src/main.rs.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use minifb::{InputCallback, Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;

const PROTO: &str = "PPSCR3";
const MAX_WIN_H: usize = 960;
const MAX_MSG: usize = 16 << 20;

// Codes clavier Linux (le téléphone les reçoit tels quels via uinput).
const KEY_BACK: u32 = 158;
const KEY_HOMEPAGE: u32 = 172;
const MOD_SHIFT: u32 = 1;
const MOD_CTRL: u32 = 2;
const MOD_ALT: u32 = 4;

fn main() {
    if let Err(e) = real_main() {
        log(&format!("erreur fatale : {e}"));
        message_box(&format!("screenview : {e}"));
        std::process::exit(1);
    }
}

fn real_main() -> io::Result<()> {
    let (mut host, mut port, mut mbps) = (None, 8099u16, 8u32);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--mbps" {
            mbps = args.next().and_then(|v| v.parse().ok()).unwrap_or(8);
        } else if let Ok(p) = a.parse::<u16>() {
            port = p;
        } else if a.len() != 32 {
            host = Some(a);
        }
    }
    let key = load_or_create_key()?;
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".into());
    log(&format!("démarrage, PC {name}"));

    // Recherche + priorité réseau en arrière-plan : la fenêtre d'attente reste fluide.
    let phone: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(host.clone()));
    if host.is_none() {
        let phone = phone.clone();
        thread::spawn(move || {
            let mut route_checked = false;
            loop {
                let found = find_phone();
                if found.is_some() && !route_checked {
                    route_checked = true;
                    fix_route_priority();
                }
                *phone.lock().unwrap() = found;
                thread::sleep(Duration::from_secs(2));
            }
        });
    }

    let mut wait = WaitWindow::new()?;
    let mut status = String::from("recherche du téléphone…");
    let mut next_try = Instant::now();
    loop {
        if !wait.show(&status) {
            return Ok(()); // fermé par l'utilisateur
        }
        if Instant::now() < next_try {
            continue;
        }
        next_try = Instant::now() + Duration::from_secs(2);
        let Some(h) = phone.lock().unwrap().clone() else {
            status = "en attente du téléphone (câble branché, Écran USB activé)".into();
            continue;
        };
        match connect(&h, port, &key, &name) {
            Ok(Some(s)) => {
                log(&format!("connecté à {h}:{port}"));
                drop(wait);
                let closed = match session(s, mbps) {
                    Ok(c) => c,
                    Err(e) => {
                        log(&format!("session : {e}"));
                        false
                    }
                };
                if closed {
                    return Ok(());
                }
                wait = WaitWindow::new()?;
                status = "connexion perdue, reprise…".into();
            }
            Ok(None) => status = "PC non autorisé : PurpleOps > Écran > APPAIRER UN PC".into(),
            Err(_) => status = format!("téléphone {h} trouvé, Écran USB pas encore activé"),
        }
    }
}

/// Petite fenêtre affichée tant qu'on n'est pas connecté ; l'état est dans le titre.
struct WaitWindow {
    win: Window,
    buf: Vec<u32>,
    title: String,
}

impl WaitWindow {
    fn new() -> io::Result<Self> {
        let (w, h) = (480, 90);
        let win = Window::new("PurpleOps — écran USB", w, h, WindowOptions::default())
            .map_err(|e| err(&format!("fenêtre : {e}")))?;
        let mut buf = vec![0x0B0A12u32; w * h];
        for x in 0..w {
            for y in 40..50 {
                buf[y * w + x] = if x % 16 < 10 { 0x9A6BFF } else { 0x0B0A12 };
            }
        }
        Ok(WaitWindow { win, buf, title: String::new() })
    }

    fn show(&mut self, status: &str) -> bool {
        if status != self.title {
            self.win.set_title(&format!("PurpleOps — {status}"));
            self.title = status.into();
        }
        self.win.update_with_buffer(&self.buf, 480, 90).is_ok() && self.win.is_open()
    }
}

/// Some(flux) si autorisé, None si ce PC n'est pas appairé.
fn connect(host: &str, port: u16, key: &str, name: &str) -> io::Result<Option<TcpStream>> {
    let sa = format!("{host}:{port}").parse().map_err(|_| err("adresse invalide"))?;
    let mut s = TcpStream::connect_timeout(&sa, Duration::from_secs(2))?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.write_all(format!("{PROTO} {key} {name}\n").as_bytes())?;
    let mut st = [0u8; 1];
    s.read_exact(&mut st)?;
    Ok((st[0] == 0).then_some(s))
}

/// Image prête à afficher, déjà à la taille de la fenêtre (0x00RRGGBB).
type Shown = Arc<Mutex<Option<Vec<u32>>>>;

#[derive(Clone, Copy)]
struct Geo {
    dw: usize, // écran du téléphone (coordonnées tactiles)
    dh: usize,
    ww: usize, // fenêtre
    wh: usize,
}

impl Geo {
    fn new(dw: usize, dh: usize) -> Self {
        let f = (dh as f32 / MAX_WIN_H as f32).max(1.0);
        Geo { dw, dh, ww: (dw as f32 / f).round() as usize, wh: (dh as f32 / f).round() as usize }
    }
    fn to_dev(self, p: (f32, f32)) -> (u32, u32) {
        let x = (p.0 * self.dw as f32 / self.ww as f32).max(0.0) as usize;
        let y = (p.1 * self.dh as f32 / self.wh as f32).max(0.0) as usize;
        (x.min(self.dw - 1) as u32, y.min(self.dh - 1) as u32)
    }
}

/// Événements envoyés au téléphone (sans réponse).
enum Ev {
    Down(u32, u32),
    Move(u32, u32),
    Up,
    Key(u32, u32),
    Text(String),
    Clip(String),
    Scroll(u32, u32, i32),
}

impl Ev {
    fn encode(&self) -> Vec<u8> {
        let mut m = Vec::new();
        let mut put = |op: u8, v: &[u32]| {
            m.push(op);
            for x in v {
                m.extend_from_slice(&x.to_be_bytes());
            }
        };
        match self {
            Ev::Down(x, y) => put(0x10, &[*x, *y]),
            Ev::Move(x, y) => put(0x11, &[*x, *y]),
            Ev::Up => put(0x12, &[]),
            Ev::Key(c, mods) => put(0x20, &[*c, *mods]),
            Ev::Text(t) => {
                put(0x21, &[t.len() as u32]);
                m.extend_from_slice(t.as_bytes());
            }
            Ev::Clip(t) => {
                put(0x22, &[t.len() as u32]);
                m.extend_from_slice(t.as_bytes());
            }
            Ev::Scroll(x, y, dy) => put(0x30, &[*x, *y, *dy as u32]),
        }
        m
    }
}

/// Caractères tapés (disposition du PC déjà appliquée par Windows).
struct Chars(Sender<char>);
impl InputCallback for Chars {
    fn add_char(&mut self, c: u32) {
        if let Some(ch) = char::from_u32(c) {
            if !ch.is_control() {
                let _ = self.0.send(ch);
            }
        }
    }
}

/// Une session : renvoie Ok(true) si l'utilisateur a fermé la fenêtre.
fn session(mut s: TcpStream, mbps: u32) -> io::Result<bool> {
    s.write_all(&[0x05])?;
    s.write_all(&(mbps.clamp(1, 40) * 1_000_000).to_be_bytes())?;
    let dw = read_u32(&mut s)? as usize;
    let dh = read_u32(&mut s)? as usize;
    if dw == 0 || dh == 0 {
        return Err(err("taille d'écran invalide"));
    }
    s.set_read_timeout(None)?;
    let geo = Geo::new(dw, dh);

    let shown: Shown = Arc::new(Mutex::new(None));
    let stats = Arc::new(Mutex::new(String::new()));
    let alive = Arc::new(Mutex::new(true));
    let (clip_tx, clip_rx) = mpsc::channel::<String>();
    {
        let (r, sh, st, al) = (s.try_clone()?, shown.clone(), stats.clone(), alive.clone());
        thread::spawn(move || {
            if let Err(e) = recv_loop(r, geo, sh, st, clip_tx) {
                log(&format!("flux interrompu : {e}"));
            }
            *al.lock().unwrap() = false;
        });
    }
    let (ev_tx, ev_rx) = mpsc::channel::<Ev>();
    {
        let mut w = s.try_clone()?;
        thread::spawn(move || {
            for e in ev_rx {
                if w.write_all(&e.encode()).is_err() {
                    break;
                }
            }
        });
    }

    let mut win = Window::new("PurpleOps — écran USB", geo.ww, geo.wh, WindowOptions::default())
        .map_err(|e| err(&format!("fenêtre : {e}")))?;
    win.limit_update_rate(Some(Duration::from_millis(6)));
    let (ch_tx, ch_rx) = mpsc::channel::<char>();
    win.set_input_callback(Box::new(Chars(ch_tx)));
    let mut buf = vec![0u32; geo.ww * geo.wh];

    let mut clip = ClipSync::new();
    let mut finger: Option<(u32, u32)> = None;
    let (mut prev_r, mut prev_m) = (false, false);
    let mut title = String::new();

    while win.is_open() {
        if !*alive.lock().unwrap() {
            return Ok(false);
        }
        // Presse-papiers : téléphone -> PC, puis PC -> téléphone (avant les touches,
        // pour qu'un Ctrl+V juste après une copie colle bien le nouveau texte).
        for t in clip_rx.try_iter() {
            clip.apply_phone(&t);
        }
        if let Some(t) = clip.pc_changed() {
            let _ = ev_tx.send(Ev::Clip(t));
        }

        // Doigt virtuel = bouton gauche : appui, déplacements, relâchement.
        let pos = win.get_mouse_pos(MouseMode::Clamp).map(|p| geo.to_dev(p));
        let left = win.get_mouse_down(MouseButton::Left);
        match (left, finger, pos) {
            (true, None, Some(p)) => {
                let _ = ev_tx.send(Ev::Down(p.0, p.1));
                finger = Some(p);
            }
            (true, Some(f), Some(p)) if f != p => {
                let _ = ev_tx.send(Ev::Move(p.0, p.1));
                finger = Some(p);
            }
            (false, Some(_), _) => {
                let _ = ev_tx.send(Ev::Up);
                finger = None;
            }
            _ => {}
        }
        if let (Some((_, dy)), Some(p)) = (win.get_scroll_wheel(), pos) {
            if dy != 0.0 && finger.is_none() {
                let px = (dy.signum() * (geo.dh as f32 / 8.0)) as i32;
                let _ = ev_tx.send(Ev::Scroll(p.0, p.1, px));
            }
        }
        let right = win.get_mouse_down(MouseButton::Right);
        if right && !prev_r {
            let _ = ev_tx.send(Ev::Key(KEY_BACK, 0));
        }
        prev_r = right;
        let middle = win.get_mouse_down(MouseButton::Middle);
        if middle && !prev_m {
            let _ = ev_tx.send(Ev::Key(KEY_HOMEPAGE, 0));
        }
        prev_m = middle;

        // Clavier : touches spéciales et raccourcis par code, texte par caractères.
        let ctrl = win.is_key_down(Key::LeftCtrl) || win.is_key_down(Key::RightCtrl);
        let alt = win.is_key_down(Key::LeftAlt) || win.is_key_down(Key::RightAlt);
        let shift = win.is_key_down(Key::LeftShift) || win.is_key_down(Key::RightShift);
        let altgr = ctrl && alt; // AltGr = Ctrl+Alt sous Windows : c'est du texte
        for k in win.get_keys_pressed(KeyRepeat::Yes) {
            let mods = (shift as u32 * MOD_SHIFT) | (ctrl as u32 * MOD_CTRL) | (alt as u32 * MOD_ALT);
            if let Some(code) = special_key(k) {
                let _ = ev_tx.send(Ev::Key(code, if altgr { 0 } else { mods }));
            } else if (ctrl || alt) && !altgr {
                if let Some(code) = letter_key(k) {
                    let _ = ev_tx.send(Ev::Key(code, mods));
                }
            }
        }
        let typed: String = ch_rx.try_iter().collect();
        if !typed.is_empty() && !((ctrl || alt) && !altgr) {
            let _ = ev_tx.send(Ev::Text(typed));
        }

        if let Some(f) = shown.lock().unwrap().take() {
            buf = f;
        }
        let st = stats.lock().unwrap().clone();
        if st != title && !st.is_empty() {
            win.set_title(&format!("PurpleOps — {st}"));
            title = st;
        }
        win.update_with_buffer(&buf, geo.ww, geo.wh).map_err(|e| err(&format!("maj fenêtre : {e}")))?;
    }
    let _ = s.shutdown(std::net::Shutdown::Both);
    Ok(true)
}

/// Messages du téléphone : u8 type + u32 taille + contenu.
/// 1 = image H.264 complète (décodée, réduite, publiée) ; 2 = presse-papiers.
fn recv_loop(mut s: TcpStream, geo: Geo, shown: Shown, stats: Arc<Mutex<String>>, clip: Sender<String>) -> io::Result<()> {
    let mut dec = Decoder::new().map_err(|e| err(&format!("décodeur H.264 : {e}")))?;
    let mut rgb: Vec<u8> = Vec::new();
    let mut msg: Vec<u8> = Vec::new();
    let (mut frames, mut errors, mut bytes, mut dec_time) = (0u32, 0u32, 0usize, Duration::ZERO);
    let mut since = Instant::now();
    loop {
        let mut head = [0u8; 5];
        s.read_exact(&mut head)?;
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if len > MAX_MSG {
            return Err(err("message annoncé trop grand"));
        }
        msg.resize(len, 0);
        s.read_exact(&mut msg)?;
        bytes += len + 5;

        match head[0] {
            1 => {
                let t0 = Instant::now();
                match dec.decode(&msg) {
                    Ok(Some(yuv)) => {
                        let (w, h) = yuv.dimensions();
                        rgb.resize(w * h * 3, 0);
                        yuv.write_rgb8(&mut rgb);
                        let mut out = vec![0u32; geo.ww * geo.wh];
                        for wy in 0..geo.wh {
                            let row = (wy * h / geo.wh) * w * 3;
                            for wx in 0..geo.ww {
                                let i = row + (wx * w / geo.ww) * 3;
                                out[wy * geo.ww + wx] = (rgb[i] as u32) << 16 | (rgb[i + 1] as u32) << 8 | rgb[i + 2] as u32;
                            }
                        }
                        *shown.lock().unwrap() = Some(out);
                        frames += 1;
                    }
                    Ok(None) => {}
                    Err(_) => errors += 1,
                }
                dec_time += t0.elapsed();
            }
            2 => {
                let _ = clip.send(String::from_utf8_lossy(&msg).into_owned());
            }
            _ => {} // type inconnu : ignoré (compatibilité future)
        }

        if since.elapsed() >= Duration::from_secs(1) {
            let secs = since.elapsed().as_secs_f64();
            let mut line = format!(
                "{:.0} img/s · {:.1} Mbit/s · décodage {:.1} ms",
                frames as f64 / secs,
                bytes as f64 * 8.0 / secs / 1e6,
                if frames > 0 { dec_time.as_secs_f64() * 1000.0 / frames as f64 } else { 0.0 }
            );
            if errors > 0 {
                line += &format!(" · {errors} erreurs");
            }
            *stats.lock().unwrap() = line;
            (frames, errors, bytes, dec_time) = (0, 0, 0, Duration::ZERO);
            since = Instant::now();
        }
    }
}

/// Synchronisation du presse-papiers Windows, sans écho : on retient le numéro de
/// séquence après chaque écriture venue du téléphone.
struct ClipSync {
    seq: u32,
    last_check: Instant,
}

impl ClipSync {
    fn new() -> Self {
        ClipSync { seq: clipboard::sequence(), last_check: Instant::now() }
    }

    fn apply_phone(&mut self, text: &str) {
        if clipboard::get().as_deref() != Some(text) && clipboard::set(text) {
            self.seq = clipboard::sequence();
        }
    }

    /// Texte copié sur le PC depuis la dernière fois (vérifié toutes les 150 ms).
    fn pc_changed(&mut self) -> Option<String> {
        if self.last_check.elapsed() < Duration::from_millis(150) {
            return None;
        }
        self.last_check = Instant::now();
        let s = clipboard::sequence();
        if s == self.seq {
            return None;
        }
        self.seq = s;
        clipboard::get().filter(|t| !t.is_empty() && t.len() < 1 << 20)
    }
}

/// Presse-papiers Windows (texte Unicode), API Win32 directe.
#[cfg(any(windows, winapi_check))]
mod clipboard {
    use std::ffi::c_void;
    type H = *mut c_void;
    const CF_UNICODETEXT: u32 = 13;
    const GMEM_MOVEABLE: u32 = 0x2;

    #[link(name = "user32")]
    extern "system" {
        fn OpenClipboard(owner: H) -> i32;
        fn CloseClipboard() -> i32;
        fn EmptyClipboard() -> i32;
        fn GetClipboardData(fmt: u32) -> H;
        fn SetClipboardData(fmt: u32, mem: H) -> H;
        fn GetClipboardSequenceNumber() -> u32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> H;
        fn GlobalLock(mem: H) -> *mut c_void;
        fn GlobalUnlock(mem: H) -> i32;
        fn GlobalFree(mem: H) -> H;
    }

    fn open() -> bool {
        for _ in 0..10 {
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        false
    }

    pub fn sequence() -> u32 {
        unsafe { GetClipboardSequenceNumber() }
    }

    pub fn get() -> Option<String> {
        if !open() {
            return None;
        }
        let r = unsafe {
            let h = GetClipboardData(CF_UNICODETEXT);
            if h.is_null() {
                None
            } else {
                let p = GlobalLock(h) as *const u16;
                if p.is_null() {
                    None
                } else {
                    let mut n = 0;
                    while *p.add(n) != 0 {
                        n += 1;
                    }
                    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                    GlobalUnlock(h);
                    Some(s)
                }
            }
        };
        unsafe { CloseClipboard() };
        r
    }

    pub fn set(text: &str) -> bool {
        let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        if !open() {
            return false;
        }
        let ok = unsafe {
            EmptyClipboard();
            let h = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
            let p = if h.is_null() { std::ptr::null_mut() } else { GlobalLock(h) as *mut u16 };
            if p.is_null() {
                false
            } else {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                GlobalUnlock(h);
                if SetClipboardData(CF_UNICODETEXT, h).is_null() {
                    GlobalFree(h);
                    false
                } else {
                    true // la mémoire appartient désormais au presse-papiers
                }
            }
        };
        unsafe { CloseClipboard() };
        ok
    }
}

#[cfg(not(any(windows, winapi_check)))]
mod clipboard {
    pub fn sequence() -> u32 {
        0
    }
    pub fn get() -> Option<String> {
        None
    }
    pub fn set(_: &str) -> bool {
        false
    }
}

/// Touches spéciales PC -> codes Linux.
fn special_key(k: Key) -> Option<u32> {
    Some(match k {
        Key::Escape => KEY_BACK,
        Key::Backspace => 14,
        Key::Tab => 15,
        Key::Enter | Key::NumPadEnter => 28,
        Key::Delete => 111,
        Key::Home => 102,
        Key::End => 107,
        Key::PageUp => 104,
        Key::PageDown => 109,
        Key::Up => 103,
        Key::Down => 108,
        Key::Left => 105,
        Key::Right => 106,
        _ => return None,
    })
}

/// Lettres pour les raccourcis Ctrl/Alt (Ctrl+A, Ctrl+C…), disposition US côté téléphone.
fn letter_key(k: Key) -> Option<u32> {
    use Key::*;
    let l = [
        (A, 30), (B, 48), (C, 46), (D, 32), (E, 18), (F, 33), (G, 34), (H, 35), (I, 23), (J, 36), (K, 37),
        (L, 38), (M, 50), (N, 49), (O, 24), (P, 25), (Q, 16), (R, 19), (S, 31), (T, 20), (U, 22), (V, 47),
        (W, 17), (X, 45), (Y, 21), (Z, 44),
    ];
    l.iter().find(|(key, _)| *key == k).map(|(_, c)| *c)
}

fn app_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("PurpleOps")
}

/// Journal simple (la fenêtre n'a pas de console).
fn log(msg: &str) {
    let _ = std::fs::create_dir_all(app_dir());
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(app_dir().join("screenview.log")) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(f, "{t} {msg}");
    }
}

/// Clé d'identification de ce PC (64 hex), créée au premier lancement.
fn load_or_create_key() -> io::Result<String> {
    let path = app_dir().join("screenview.key");
    if let Ok(k) = std::fs::read_to_string(&path) {
        let k = k.trim().to_string();
        if k.len() == 64 && k.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(k);
        }
    }
    std::fs::create_dir_all(app_dir())?;
    let k: String = random32()?.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::write(&path, &k)?;
    Ok(k)
}

#[cfg(windows)]
fn random32() -> io::Result<[u8; 32]> {
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(alg: *mut std::ffi::c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
    }
    let mut b = [0u8; 32];
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG = 2
    if unsafe { BCryptGenRandom(std::ptr::null_mut(), b.as_mut_ptr(), 32, 2) } != 0 {
        return Err(err("générateur aléatoire Windows indisponible"));
    }
    Ok(b)
}

#[cfg(not(windows))]
fn random32() -> io::Result<[u8; 32]> {
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b)
}

/// PowerShell sans fenêtre ; renvoie sa sortie.
fn powershell(script: &str) -> Option<String> {
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

const NCM_FILTER: &str = "Where-Object { $_.InterfaceDescription -like '*NCM*' -or $_.InterfaceDescription -like '*NDIS*' }";

/// Adresse du téléphone = passerelle que son DHCP a donnée à la carte UsbNcm.
fn find_phone() -> Option<String> {
    let s = powershell(&format!(
        "(Get-NetIPConfiguration | {NCM_FILTER} | Select-Object -First 1).IPv4DefaultGateway.NextHop"
    ))?;
    s.parse::<std::net::Ipv4Addr>().ok().map(|ip| ip.to_string())
}

/// Le partage de connexion d'Android donne aussi au PC une route Internet par le
/// téléphone ; la carte USB (426 Mbit/s) passe alors devant le Wi-Fi. On la met en
/// dernière priorité (métrique 5000) : une confirmation administrateur, une fois.
fn fix_route_priority() {
    let metric = powershell(&format!(
        "(Get-NetAdapter | {NCM_FILTER} | Get-NetIPInterface -AddressFamily IPv4 | Select-Object -First 1).InterfaceMetric"
    ));
    let m: u32 = metric.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0);
    if m >= 1000 {
        return;
    }
    log(&format!("carte USB du téléphone prioritaire (métrique {m}) : correction demandée"));
    let inner = format!("Get-NetAdapter | {NCM_FILTER} | Set-NetIPInterface -InterfaceMetric 5000");
    let encoded = b64_utf16(&inner);
    let _ = powershell(&format!(
        "Start-Process powershell -Verb RunAs -WindowStyle Hidden -ArgumentList '-NoProfile','-EncodedCommand','{encoded}'"
    ));
}

/// Base64 d'un texte en UTF-16LE (format de -EncodedCommand).
fn b64_utf16(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let d: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let mut o = String::new();
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            o.push(if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    o
}

#[cfg(windows)]
fn message_box(text: &str) {
    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    let t: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let c: Vec<u16> = "PurpleOps".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), 0x10) };
}

#[cfg(not(windows))]
fn message_box(text: &str) {
    eprintln!("{text}");
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
    fn geo_maps_corners() {
        let g = Geo::new(1080, 2400);
        assert_eq!(g.wh, MAX_WIN_H);
        assert_eq!(g.to_dev((0.0, 0.0)), (0, 0));
        assert_eq!(g.to_dev((g.ww as f32, g.wh as f32)), (1079, 2399));
    }

    #[test]
    fn events_encode() {
        assert_eq!(Ev::Up.encode(), vec![0x12]);
        assert_eq!(Ev::Down(1, 2).encode(), vec![0x10, 0, 0, 0, 1, 0, 0, 0, 2]);
        assert_eq!(Ev::Clip("é".into()).encode(), vec![0x22, 0, 0, 0, 2, 0xc3, 0xa9]);
        assert_eq!(Ev::Scroll(0, 0, -1).encode()[9..], [0xff, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn encoded_command() {
        // "a" en UTF-16LE = 61 00 -> "YQA="
        assert_eq!(b64_utf16("a"), "YQA=");
    }
}
