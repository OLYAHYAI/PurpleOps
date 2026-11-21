//! Page d'accueil de screenview (thème « Neon Ops » du téléphone).
//!
//! Affichée au lancement, après F12 ou quand l'image est perdue :
//! - état de la liaison (téléphone joignable, appairage, latence) et connexion
//!   automatique après un compte à rebours de 3 s (sauf « Rester ici » ou perte d'image) ;
//! - secours SANS toucher au téléphone : « Récupérer l'écran du téléphone » (coupe la
//!   session en cours, même bloquée, rétablit veille et luminosité, réveille l'écran)
//!   et « Couper le pilotage IA (MCP) » ; commandes signées par la clé d'appairage
//!   de ce PC, tracées dans le journal SOC du téléphone (Wazuh).
//!
//! Le téléphone est interrogé (commande STATUS) toutes les 5 s, dans un fil à part.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};

use crate::ui::{self, Btn, Canvas, Fonts, Rect};
use crate::{connect_raw, err, log, set_window_icon, PROTO};

pub const W: usize = 480;
pub const H: usize = 760;
const AUTO_DELAY: Duration = Duration::from_secs(3);
const PROBE_EVERY: Duration = Duration::from_secs(5);

/// Ce que le téléphone a répondu au dernier sondage.
#[derive(Clone, Debug, PartialEq)]
pub enum Probe {
    /// Aucun hôte connu (ni dernier hôte, ni téléphone USB trouvé).
    Searching,
    /// Hôte connu mais injoignable (service Écran arrêté, Tailscale coupé…).
    Unreachable { host: String },
    /// Joignable, mais ce PC n'est pas appairé.
    NotPaired { host: String, ms: u32 },
    /// Joignable et appairé ; `status` absent = ancien service (à mettre à jour).
    Ready { host: String, ms: u32, status: Option<Status> },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Status {
    pub busy: Option<String>,
    pub dim: bool,
    pub ia: String,
}

impl Status {
    /// « STATUS v=2 session=busy host=PC-A dim=1 ia=on »
    pub fn parse(line: &str) -> Option<Status> {
        let rest = line.trim().strip_prefix("STATUS ")?;
        let mut st = Status { ia: "absent".into(), ..Default::default() };
        let (mut busy, mut host) = (false, String::new());
        for kv in rest.split_whitespace() {
            let (k, v) = kv.split_once('=')?;
            match k {
                "session" => busy = v == "busy",
                "host" => host = v.to_string(),
                "dim" => st.dim = v == "1",
                "ia" => st.ia = v.to_string(),
                _ => {}
            }
        }
        st.busy = busy.then_some(host);
        Some(st)
    }
}

/// Pourquoi on revient à l'accueil.
pub enum Entry {
    Start,
    /// F12 pendant la session.
    User,
    /// Image perdue / connexion coupée (raison affichée).
    Lost(String),
    /// Écran récupéré depuis la barre de la session (résultat affiché).
    Rescued(String),
}

pub enum Exit {
    Quit,
    Connected(TcpStream, String),
}

/// Commande de secours envoyée au téléphone (opcodes de screenctl).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Cmd {
    Rescue = 0x06,
    Status = 0x07,
    IaOff = 0x08,
}

/// Envoie une commande de secours ; renvoie la ligne de réponse.
/// `Ok(None)` : PC non appairé ; réponse vide : ancien service (commande inconnue).
pub(crate) fn command(host: &str, port: u16, key: &str, name: &str, cmd: Cmd) -> io::Result<Option<String>> {
    let Some(mut s) = connect_raw(host, port, key, name, Duration::from_secs(2))? else { return Ok(None) };
    s.set_read_timeout(Some(Duration::from_secs(if cmd == Cmd::Rescue { 10 } else { 4 })))?;
    s.write_all(&[cmd as u8])?;
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    while line.len() < 400 {
        match s.read(&mut b) {
            Ok(0) => break,
            Ok(_) if b[0] == b'\n' => break,
            Ok(_) => line.push(b[0]),
            Err(e) if line.is_empty() && matches!(e.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

fn probe(host: Option<String>, port: u16, key: &str, name: &str) -> Probe {
    let Some(host) = host else { return Probe::Searching };
    let t0 = Instant::now();
    match command(&host, port, key, name, Cmd::Status) {
        Ok(None) => Probe::NotPaired { ms: t0.elapsed().as_millis() as u32, host },
        Ok(Some(line)) => Probe::Ready { ms: t0.elapsed().as_millis() as u32, status: Status::parse(&line), host },
        Err(_) => Probe::Unreachable { host },
    }
}

/// Simple joignabilité (ancien service) : connexion TCP sans poignée de main.
fn reach(host: &str, port: u16) -> Probe {
    let t0 = Instant::now();
    match format!("{host}:{port}").parse() {
        Ok(sa) if TcpStream::connect_timeout(&sa, Duration::from_secs(2)).is_ok() => {
            Probe::Ready { host: host.to_string(), ms: t0.elapsed().as_millis() as u32, status: None }
        }
        _ => Probe::Unreachable { host: host.to_string() },
    }
}

struct Shared {
    probe: Probe,
    journal: Vec<String>,
    /// Accueil affiché : on ne sonde pas le téléphone pendant une session d'image.
    visible: bool,
}

pub struct Home {
    phone: Arc<Mutex<Option<String>>>,
    port: u16,
    key: String,
    name: String,
    shared: Arc<Mutex<Shared>>,
    wake: Sender<()>,
}

impl Home {
    pub fn new(phone: Arc<Mutex<Option<String>>>, port: u16, key: String, name: String) -> Home {
        let shared = Arc::new(Mutex::new(Shared { probe: Probe::Searching, journal: Vec::new(), visible: true }));
        let (wake, wake_rx) = mpsc::channel::<()>();
        {
            let (phone, key, name, shared) = (phone.clone(), key.clone(), name.clone(), shared.clone());
            thread::spawn(move || {
                // Ancien service (STATUS inconnu) : il journalise chaque commande
                // inconnue ; on ne lui repose la question qu'une fois par minute et,
                // entre-temps, on vérifie seulement qu'il répond (connexion TCP).
                let mut outdated_since: Option<Instant> = None;
                loop {
                    if shared.lock().unwrap().visible {
                        let host = phone.lock().unwrap().clone();
                        let p = match (&host, outdated_since) {
                            (Some(h), Some(t)) if t.elapsed() < Duration::from_secs(60) => reach(h, port),
                            _ => probe(host, port, &key, &name),
                        };
                        match &p {
                            Probe::Ready { status: None, .. } => {
                                outdated_since.get_or_insert_with(Instant::now);
                            }
                            Probe::Ready { status: Some(_), .. } => outdated_since = None,
                            _ => {}
                        }
                        if outdated_since.is_some_and(|t| t.elapsed() >= Duration::from_secs(60)) {
                            outdated_since = None; // reposer STATUS au prochain tour
                        }
                        shared.lock().unwrap().probe = p;
                    }
                    // Prochain sondage : 5 s, ou tout de suite après une commande.
                    let _ = wake_rx.recv_timeout(PROBE_EVERY);
                }
            });
        }
        Home { phone, port, key, name, shared, wake }
    }

    fn note(&self, line: &str) {
        log(line);
        let t = clock();
        let mut sh = self.shared.lock().unwrap();
        sh.journal.push(format!("[{t}] {line}"));
        let n = sh.journal.len();
        if n > 4 {
            sh.journal.drain(..n - 4);
        }
    }

    /// Commande de secours dans un fil (la fenêtre reste fluide).
    fn run_command(&self, cmd: Cmd, busy: Arc<Mutex<bool>>) {
        let Some(host) = self.phone.lock().unwrap().clone() else {
            self.note("aucun téléphone connu");
            return;
        };
        *busy.lock().unwrap() = true;
        let (port, key, name, wake) = (self.port, self.key.clone(), self.name.clone(), self.wake.clone());
        let shared = self.shared.clone();
        thread::spawn(move || {
            let what = if cmd == Cmd::Rescue { "secours" } else { "pilotage IA" };
            let msg = match command(&host, port, &key, &name, cmd) {
                Ok(None) => format!("{what} : PC non appairé"),
                Ok(Some(l)) if l.is_empty() => format!("{what} : téléphone à mettre à jour (commande inconnue)"),
                Ok(Some(l)) => format!("{what} : {}", humanize(&l)),
                Err(e) => format!("{what} : téléphone injoignable ({e})"),
            };
            log(&msg);
            {
                let mut sh = shared.lock().unwrap();
                sh.journal.push(format!("[{}] {msg}", clock()));
                let n = sh.journal.len();
                if n > 4 {
                    sh.journal.drain(..n - 4);
                }
            }
            *busy.lock().unwrap() = false;
            let _ = wake.send(());
        });
    }

    /// Boucle de la page d'accueil, jusqu'à une connexion ou la fermeture.
    pub fn run(&mut self, fonts: &mut Fonts, entry: Entry) -> io::Result<Exit> {
        self.shared.lock().unwrap().visible = true;
        let _ = self.wake.send(()); // sondage immédiat
        let res = self.run_inner(fonts, entry);
        self.shared.lock().unwrap().visible = false;
        res
    }

    fn run_inner(&mut self, fonts: &mut Fonts, entry: Entry) -> io::Result<Exit> {
        let mut win = Window::new("PurpleOps — screenview", W, H, WindowOptions::default())
            .map_err(|e| err(&format!("fenêtre : {e}")))?;
        set_window_icon(&win);
        win.limit_update_rate(Some(Duration::from_millis(16)));

        let lost = match &entry {
            Entry::Lost(why) => {
                self.note(&format!("retour à l'accueil : {why}"));
                Some(why.clone())
            }
            Entry::User => {
                self.note("retour à l'accueil (F12)");
                None
            }
            Entry::Rescued(msg) => {
                self.note(&msg);
                None
            }
            Entry::Start => None,
        };
        // Connexion automatique : au lancement seulement (pas après F12 ni perte d'image).
        let mut auto = matches!(entry, Entry::Start);
        let mut ready_since: Option<Instant> = None;
        let mut confirm_ia: Option<Instant> = None;
        let busy = Arc::new(Mutex::new(false));
        let mut connecting: Option<Receiver<io::Result<Option<TcpStream>>>> = None;
        let mut connect_err: Option<String> = None;
        let mut pressed: Option<usize> = None;
        let mut canvas = Canvas::new(W, H);

        while win.is_open() {
            let probe = self.shared.lock().unwrap().probe.clone();
            let journal = self.shared.lock().unwrap().journal.clone();
            let is_busy = *busy.lock().unwrap();
            let paired_ready = matches!(probe, Probe::Ready { .. });

            // Compte à rebours de la connexion automatique.
            if paired_ready && auto && connecting.is_none() {
                ready_since.get_or_insert_with(Instant::now);
            } else {
                ready_since = None;
            }
            let remaining = ready_since.map(|t| AUTO_DELAY.saturating_sub(t.elapsed()));
            let mut want_connect = remaining.is_some_and(|r| r.is_zero());

            // Résultat d'une connexion lancée.
            if let Some(rx) = &connecting {
                if let Ok(res) = rx.try_recv() {
                    connecting = None;
                    match res {
                        Ok(Some(s)) => {
                            let host = self.phone.lock().unwrap().clone().unwrap_or_default();
                            return Ok(Exit::Connected(s, host));
                        }
                        Ok(None) => connect_err = Some("PC non autorisé : PurpleOps › Écran › Appairer un PC".into()),
                        Err(e) => connect_err = Some(format!("connexion impossible : {e}")),
                    }
                    if let Some(e) = &connect_err {
                        self.note(e);
                    }
                    auto = false;
                }
            }
            if confirm_ia.is_some_and(|t| t.elapsed() > Duration::from_secs(4)) {
                confirm_ia = None;
            }

            // ---------------------------------------------------------- dessin
            let layout = draw(
                &mut canvas, fonts, &self.name, &probe, lost.as_deref(), remaining, auto,
                connecting.is_some(), connect_err.as_deref(), confirm_ia.is_some(), is_busy, &journal,
            );

            // ---------------------------------------------------------- souris
            let mouse = win.get_mouse_pos(MouseMode::Discard);
            let hover = mouse.and_then(|p| layout.buttons.iter().position(|b| b.enabled && b.rect.contains(p)));
            if let Some(i) = hover {
                let b = &layout.buttons[i];
                canvas.button(fonts, b.rect, &b.label, b.kind, true, true);
            }
            let down = win.get_mouse_down(MouseButton::Left);
            let mut clicked: Option<Action> = None;
            if down && pressed.is_none() {
                pressed = Some(hover.unwrap_or(usize::MAX));
            } else if !down {
                if let Some(p) = pressed.take() {
                    if hover == Some(p) {
                        clicked = Some(layout.buttons[p].action);
                    }
                }
            }
            for k in win.get_keys_pressed(KeyRepeat::No) {
                match k {
                    Key::Enter | Key::NumPadEnter if !is_busy => clicked = Some(Action::Connect),
                    Key::Escape => auto = false,
                    _ => {}
                }
            }
            match clicked {
                Some(Action::Connect) => want_connect = true,
                Some(Action::Stay) => {
                    auto = false;
                    self.note("connexion automatique annulée");
                }
                Some(Action::Auto) => {
                    auto = true;
                    connect_err = None;
                }
                Some(Action::Rescue) => {
                    self.note("secours : récupération de l'écran du téléphone…");
                    self.run_command(Cmd::Rescue, busy.clone());
                }
                Some(Action::IaOff) => {
                    if confirm_ia.is_some() {
                        confirm_ia = None;
                        self.note("pilotage IA : coupure demandée…");
                        self.run_command(Cmd::IaOff, busy.clone());
                    } else {
                        confirm_ia = Some(Instant::now());
                    }
                }
                None => {}
            }
            if want_connect && connecting.is_none() {
                if let Some(host) = self.phone.lock().unwrap().clone() {
                    let (tx, rx) = mpsc::channel();
                    let (port, key, name) = (self.port, self.key.clone(), self.name.clone());
                    thread::spawn(move || {
                        let _ = tx.send(connect_raw(&host, port, &key, &name, Duration::from_secs(3)));
                    });
                    connecting = Some(rx);
                    connect_err = None;
                    ready_since = None;
                } else {
                    connect_err = Some("aucun téléphone trouvé".into());
                }
            }

            win.update_with_buffer(&canvas.buf, W, H).map_err(|e| err(&format!("maj fenêtre : {e}")))?;
        }
        Ok(Exit::Quit)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Connect,
    Stay,
    Auto,
    Rescue,
    IaOff,
}

struct Button {
    rect: Rect,
    label: String,
    kind: Btn,
    action: Action,
    enabled: bool,
}

struct Layout {
    buttons: Vec<Button>,
}

/// Réponse brute du téléphone -> texte lisible.
pub(crate) fn humanize(l: &str) -> String {
    let l = l.trim();
    if let Some(r) = l.strip_prefix("OK ") {
        let mut out = Vec::new();
        for kv in r.split_whitespace() {
            match kv.split_once('=') {
                Some(("session", "aucune")) => out.push("aucune session à couper".to_string()),
                Some(("session", v)) => out.push(format!("session coupée {}", v.trim_start_matches("coupee"))),
                Some(("luminosite", "retablie")) => out.push("luminosité rétablie".into()),
                Some(("ecran", _)) => out.push("écran réveillé".into()),
                Some(("ia", "off")) => out.push("pilotage IA coupé".into()),
                Some(("ia", "absent")) => out.push("pilotage IA non activé sur le téléphone".into()),
                _ => {}
            }
        }
        if out.is_empty() { "ok".into() } else { out.join(", ") }
    } else {
        l.to_string()
    }
}

fn clock() -> String {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let local = s as i64 + local_offset_secs();
    let d = local.rem_euclid(86_400);
    format!("{:02}:{:02}:{:02}", d / 3600, d % 3600 / 60, d % 60)
}

#[cfg(windows)]
fn local_offset_secs() -> i64 {
    #[repr(C)]
    struct Tzi {
        bias: i32,
        rest: [u8; 168],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTimeZoneInformation(tzi: *mut Tzi) -> u32;
    }
    let mut t = Tzi { bias: 0, rest: [0; 168] };
    // Biais en minutes (UTC = local + biais) ; l'heure d'été est ajoutée par Windows
    // dans DaylightBias (rest) : on s'en tient au biais de base + 0/−60 selon le code.
    let code = unsafe { GetTimeZoneInformation(&mut t) };
    let dst = if code == 2 { 60 } else { 0 };
    -(t.bias as i64) * 60 + dst * 60
}

#[cfg(not(windows))]
fn local_offset_secs() -> i64 {
    0
}

/// Dessine toute la page ; renvoie les zones cliquables.
#[allow(clippy::too_many_arguments)]
fn draw(
    c: &mut Canvas,
    f: &mut Fonts,
    pc: &str,
    probe: &Probe,
    lost: Option<&str>,
    remaining: Option<Duration>,
    auto: bool,
    connecting: bool,
    connect_err: Option<&str>,
    confirm_ia: bool,
    busy: bool,
    journal: &[String],
) -> Layout {
    use ui::*;
    let mut buttons = Vec::new();
    let x = 22.0;
    let cw = W as f32 - 2.0 * x;
    c.background(if lost.is_some() { 0x2A0F18 } else { BG_TOP });

    // En-tête : logo, nom, PC, pastille d'appairage.
    c.logo(x + 17.0, 44.0, 17.0);
    c.text(f, Face::Title, 19.0, x + 46.0, 44.0, TEXT, "PURPLEOPS");
    c.kicker(f, x + 46.0, 62.0, MUTED, &format!("SCREENVIEW · PC {}", pc.to_uppercase()));
    let (chip, col, light) = match probe {
        Probe::Ready { status: None, .. } => ("À METTRE À JOUR", WARN, WARN_SOFT),
        Probe::Ready { .. } => ("APPAIRÉ", OK, OK_SOFT),
        Probe::NotPaired { .. } => ("NON APPAIRÉ", WARN, WARN_SOFT),
        Probe::Unreachable { .. } => ("INJOIGNABLE", CRIT, CRIT_SOFT),
        Probe::Searching => ("RECHERCHE…", ACCENT, ACCENT_SOFT),
    };
    if lost.is_some() {
        c.chip(f, x + cw, 30.0, "IMAGE PERDUE", CRIT, CRIT_SOFT, false);
    } else {
        c.chip(f, x + cw, 30.0, chip, col, light, true);
    }

    // ---------------------------------------------------------------- carte liaison
    let (cy, ch) = (88.0, 244.0);
    let (edge, top, glowc) = if lost.is_some() { (CRIT, 0x301018, CRIT) } else { (ACCENT, FOCUS, ACCENT) };
    c.glow(x, cy, cw, ch, 16.0, glowc, 0.5, 14.0);
    c.rrect_grad(x, cy, cw, ch, 16.0, top, 0x0D1117, 0.95);
    c.rrect(x, cy, cw, ch, 16.0, None, Some((edge, 0.4)));

    // PC ── mode ── téléphone
    let iy = cy + 20.0;
    c.rrect(x + 18.0, iy, 48.0, 48.0, 14.0, Some((BG, 0.7)), Some((BORDER, 1.0)));
    c.rrect(x + 30.0, iy + 15.0, 24.0, 15.0, 2.5, None, Some((ACCENT_SOFT, 1.0)));
    c.rrect(x + 37.0, iy + 34.0, 10.0, 1.6, 0.8, Some((ACCENT_SOFT, 1.0)), None);
    let phone_ok = matches!(probe, Probe::Ready { .. } | Probe::NotPaired { .. });
    let pc_col = if phone_ok { OK } else { BORDER_STRONG };
    let px0 = x + cw - 66.0;
    c.rrect(px0, iy, 48.0, 48.0, 14.0, Some((if phone_ok { OK } else { BG }, if phone_ok { 0.08 } else { 0.7 })), Some((pc_col, 1.0)));
    c.rrect(px0 + 18.0, iy + 12.0, 12.0, 24.0, 2.5, None, Some((if phone_ok { OK_SOFT } else { FAINT }, 1.0)));
    let mode = match probe {
        Probe::Ready { host, .. } | Probe::NotPaired { host, .. } | Probe::Unreachable { host } => {
            if host.starts_with("100.") { "TAILSCALE · :8099" } else { "USB · :8099" }
        }
        Probe::Searching => "RECHERCHE",
    };
    let mw = f.width(Face::Mono, 11.0, mode);
    let mid = x + cw / 2.0;
    c.hline_dashed(x + 76.0, mid - mw / 2.0 - 8.0, iy + 24.0, if phone_ok { ACCENT } else { BORDER_STRONG });
    c.hline_dashed(mid + mw / 2.0 + 8.0, px0 - 10.0, iy + 24.0, if phone_ok { ACCENT } else { BORDER_STRONG });
    c.text(f, Face::Mono, 11.0, mid - mw / 2.0, iy + 28.0, ACCENT_SOFT, mode);

    // État du téléphone
    let (title, sub, ms) = match (lost, probe) {
        (Some(why), _) => ("Connexion à l'écran perdue".to_string(), why.to_string(), None),
        (None, Probe::Searching) => ("Recherche du téléphone…".into(), "câble USB branché, ou Écran activé via Tailscale".into(), None),
        (None, Probe::Unreachable { host }) => ("Téléphone injoignable".into(), format!("{host} · Écran arrêté ou Tailscale coupé"), None),
        (None, Probe::NotPaired { host, ms }) => ("PC non appairé".into(), format!("{host} · PurpleOps › Écran › Appairer un PC"), Some(*ms)),
        (None, Probe::Ready { host, ms, .. }) => ("Téléphone joignable".into(), format!("{host} · dernier hôte connu"), Some(*ms)),
    };
    let ty = iy + 86.0;
    c.text(f, Face::Title, 16.0, x + 18.0, ty, if lost.is_some() { 0xFFD0D4 } else { TEXT }, &title);
    let sub = f.fit(Face::Mono, 11.0, &sub, cw - 100.0);
    c.text(f, Face::Mono, 11.0, x + 18.0, ty + 19.0, MUTED, &sub);
    if let Some(ms) = ms {
        c.text_right(f, Face::Mono, 12.0, x + cw - 18.0, ty, if ms < 150 { OK_SOFT } else { WARN_SOFT }, &format!("{ms} ms"));
    }

    // Compte à rebours / état de la connexion
    let ry = ty + 48.0;
    let (line, frac, accent) = if connecting {
        ("Connexion en cours…".to_string(), 1.0, ACCENT)
    } else if let Some(e) = connect_err {
        (e.to_string(), 0.0, CRIT)
    } else if let Some(r) = remaining {
        (format!("Connexion automatique dans {} s", r.as_secs() + 1), 1.0 - r.as_secs_f32() / AUTO_DELAY.as_secs_f32(), ACCENT)
    } else if lost.is_some() {
        ("Connexion automatique en pause : récupère l'écran ci-dessous".into(), 0.0, CRIT)
    } else if !auto {
        ("Connexion automatique désactivée".into(), 0.0, MUTED)
    } else {
        ("Connexion automatique dès que le téléphone répond".into(), 0.0, MUTED)
    };
    let line = f.fit(Face::Body, 12.5, &line, cw - 36.0);
    c.text(f, Face::Body, 12.5, x + 18.0, ry, if accent == CRIT { CRIT_SOFT } else { SOFT }, &line);
    c.bar(x + 18.0, ry + 9.0, cw - 36.0, frac, accent);

    let by = ry + 26.0;
    let bw1 = (cw - 36.0 - 10.0) * 0.58;
    let r1 = Rect { x: x + 18.0, y: by, w: bw1, h: 46.0 };
    let r2 = Rect { x: x + 18.0 + bw1 + 10.0, y: by, w: cw - 36.0 - bw1 - 10.0, h: 46.0 };
    let can_connect = !connecting && !matches!(probe, Probe::Searching | Probe::Unreachable { .. });
    buttons.push(Button { rect: r1, label: if lost.is_some() { "Reconnecter".into() } else { "Se connecter".into() }, kind: Btn::Primary, action: Action::Connect, enabled: can_connect });
    if auto && lost.is_none() {
        buttons.push(Button { rect: r2, label: "Rester ici".into(), kind: Btn::Ghost, action: Action::Stay, enabled: true });
    } else {
        buttons.push(Button { rect: r2, label: "Connexion auto".into(), kind: Btn::Ghost, action: Action::Auto, enabled: true });
    }

    // ---------------------------------------------------------------- carte secours
    let (sy, sh) = (cy + ch + 16.0, 262.0);
    c.rrect(x, sy, cw, sh, 16.0, Some((SURFACE, 0.9)), Some((BORDER, 1.0)));
    c.text(f, Face::Title, 15.0, x + 16.0, sy + 28.0, TEXT, "Secours");
    c.text_right(f, Face::Mono, 11.0, x + cw - 16.0, sy + 27.0, MUTED, "sans toucher au téléphone");

    let status = match probe {
        Probe::Ready { status, .. } => status.clone(),
        _ => None,
    };
    let known = status.is_some();
    let tw = (cw - 32.0 - 8.0) / 2.0;
    let tiles: [(&str, String, u32); 2] = match &status {
        Some(s) => [
            ("Session écran", match &s.busy { Some(h) => format!("occupée · {h}"), None => "libre".into() }, if s.busy.is_some() { WARN } else { OK }),
            ("Pilotage IA (MCP)", match s.ia.as_str() { "on" => "autorisé".into(), "off" => "coupé".into(), _ => "non activé".into() }, if s.ia == "on" { WARN } else { OK }),
        ],
        None => [("Session écran", "inconnue".into(), BORDER_STRONG), ("Pilotage IA (MCP)", "inconnu".into(), BORDER_STRONG)],
    };
    for (i, (k, v, col)) in tiles.iter().enumerate() {
        let tx = x + 16.0 + i as f32 * (tw + 8.0);
        let warn = *col == WARN;
        c.rrect(tx, sy + 42.0, tw, 54.0, 10.0, Some((if warn { WARN } else { BG }, if warn { 0.07 } else { 0.6 })), Some((if warn { WARN } else { BORDER }, if warn { 0.45 } else { 1.0 })));
        c.text(f, Face::Body, 11.0, tx + 10.0, sy + 60.0, MUTED, k);
        c.dot(tx + 14.0, sy + 79.0, 3.5, *col, false);
        let light = match *col { OK => OK_SOFT, WARN => WARN_SOFT, _ => FAINT };
        let v = f.fit(Face::Mono, 12.0, v, tw - 34.0);
        c.text(f, Face::Mono, 12.0, tx + 24.0, sy + 83.0, light, &v);
    }

    let rr = Rect { x: x + 16.0, y: sy + 108.0, w: cw - 32.0, h: 52.0 };
    buttons.push(Button {
        rect: rr,
        label: if busy { "Commande en cours…".into() } else { "Récupérer l'écran du téléphone".into() },
        kind: Btn::Danger,
        action: Action::Rescue,
        enabled: known && !busy,
    });
    let expl = if known {
        "Coupe la session (même bloquée), rallume l'écran et sa luminosité."
    } else if matches!(probe, Probe::Ready { status: None, .. }) {
        "Le téléphone ne connaît pas encore ces commandes : mets à jour PurpleOps."
    } else {
        "Disponible quand le téléphone répond et que ce PC est appairé."
    };
    c.text(f, Face::Body, 11.5, x + 16.0, sy + 180.0, MUTED, &f.fit(Face::Body, 11.5, expl, cw - 32.0));
    c.text(f, Face::Body, 11.5, x + 16.0, sy + 196.0, FAINT, "Signé par la clé de ce PC, tracé dans Wazuh.");
    let ri = Rect { x: x + 16.0, y: sy + 208.0, w: cw - 32.0, h: 40.0 };
    let ia_on = status.as_ref().is_some_and(|s| s.ia == "on");
    buttons.push(Button {
        rect: ri,
        label: if confirm_ia { "Confirmer : couper le pilotage IA".into() } else { "Couper le pilotage IA (MCP)…".into() },
        kind: if confirm_ia { Btn::Danger } else { Btn::OutlineDanger },
        action: Action::IaOff,
        enabled: ia_on && !busy,
    });

    // ---------------------------------------------------------------- journal
    let jy = sy + sh + 14.0;
    let jh = 92.0;
    c.rrect(x, jy, cw, jh, 14.0, Some((CONSOLE, 1.0)), Some((BORDER, 1.0)));
    c.kicker(f, x + 12.0, jy + 18.0, MUTED, "JOURNAL · SCREENVIEW");
    let lines: Vec<&String> = journal.iter().rev().take(3).collect();
    for (i, l) in lines.iter().rev().enumerate() {
        let t = f.fit(Face::Mono, 11.0, l, cw - 24.0);
        let col = if l.contains("injoignable") || l.contains("impossible") || l.contains("perdue") { WARN_SOFT } else { CONSOLE_TEXT };
        c.text(f, Face::Mono, 11.0, x + 12.0, jy + 40.0 + i as f32 * 17.0, col, &t);
    }
    if journal.is_empty() {
        c.text(f, Face::Mono, 11.0, x + 12.0, jy + 40.0, FAINT, "prêt.");
    }

    // ---------------------------------------------------------------- pied
    let fy = H as f32 - 18.0;
    c.text(f, Face::Mono, 11.0, x, fy, FAINT, "F12 · revenir ici");
    c.text_center(f, Face::Mono, 11.0, x, cw, fy, FAINT, "Échap · Retour");
    c.text_right(f, Face::Mono, 11.0, x + cw, fy, FAINT, concat!("v", env!("CARGO_PKG_VERSION")));

    for b in &buttons {
        c.button(f, b.rect, &b.label, b.kind, false, b.enabled);
    }
    let _ = PROTO;
    Layout { buttons }
}

/// Rendu de la page dans un tampon, sans fenêtre (aperçus et tests).
#[allow(dead_code)]
pub fn render_preview(probe: &Probe, lost: Option<&str>, journal: &[String]) -> Canvas {
    let mut fonts = Fonts::load();
    let mut c = Canvas::new(W, H);
    let remaining = if lost.is_none() && matches!(probe, Probe::Ready { .. }) { Some(Duration::from_millis(1800)) } else { None };
    draw(&mut c, &mut fonts, "LENOVO", probe, lost, remaining, lost.is_none(), false, None, false, false, journal);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parse() {
        let s = Status::parse("STATUS v=2 session=busy host=PC-A dim=1 ia=on").unwrap();
        assert_eq!(s.busy.as_deref(), Some("PC-A"));
        assert!(s.dim);
        assert_eq!(s.ia, "on");
        let s = Status::parse("STATUS v=2 session=free host=- dim=0 ia=absent\n").unwrap();
        assert_eq!(s.busy, None);
        assert!(Status::parse("").is_none());
        assert!(Status::parse("OK ia=off").is_none());
    }

    #[test]
    fn humanize_rescue() {
        let h = humanize("OK session=coupee(PC-B) luminosite=retablie ecran=reveille");
        assert!(h.contains("session coupée (PC-B)"), "{h}");
        assert!(h.contains("luminosité rétablie"));
        assert_eq!(humanize("OK ia=off"), "pilotage IA coupé");
    }

    /// Aperçus PNG de la page (`cargo test -- --ignored previews`), pour relecture.
    #[test]
    #[ignore]
    fn previews() {
        let ok = Probe::Ready {
            host: "100.84.12.7".into(),
            ms: 38,
            status: Some(Status { busy: None, dim: true, ia: "on".into() }),
        };
        let j = vec!["[10:58:02] démarrage, PC LENOVO".to_string()];
        write_ppm("/tmp/screenview-accueil.ppm", &render_preview(&ok, None, &j));
        let busy = Probe::Ready {
            host: "100.84.12.7".into(),
            ms: 41,
            status: Some(Status { busy: Some("LENOVO".into()), dim: true, ia: "on".into() }),
        };
        let j2 = vec![
            "[11:02:40] retour à l'accueil : plus de signal du téléphone depuis 8 s".to_string(),
            "[11:02:44] secours : session coupée (LENOVO), luminosité rétablie, écran réveillé".to_string(),
        ];
        write_ppm("/tmp/screenview-perdu.ppm", &render_preview(&busy, Some("plus de signal du téléphone depuis 8 s"), &j2));
    }

    /// Bout en bout contre un screenctl local :
    /// `screenctl pair 300 --dir D ; screenctl serve --listen 127.0.0.1:18099 --dir D --ctl D/ctl.sock`
    /// puis `cargo test -- --ignored e2e_local`.
    #[test]
    #[ignore]
    fn e2e_local() {
        let key = "b".repeat(64);
        let p = probe(Some("127.0.0.1".into()), 18099, &key, "PC-TEST");
        let Probe::Ready { status: Some(st), .. } = p else { panic!("{p:?}") };
        assert_eq!(st.ia, "on");
        let r = command("127.0.0.1", 18099, &key, "PC-TEST", Cmd::Rescue).unwrap().unwrap();
        assert!(r.starts_with("OK session="), "{r}");
        let r = command("127.0.0.1", 18099, &key, "PC-TEST", Cmd::IaOff).unwrap().unwrap();
        assert_eq!(humanize(&r), "pilotage IA coupé");
        let Probe::Ready { status: Some(st), .. } = probe(Some("127.0.0.1".into()), 18099, &key, "PC-TEST") else { panic!() };
        assert_eq!(st.ia, "off");
    }

    fn write_ppm(path: &str, c: &Canvas) {
        let mut out = format!("P6\n{} {}\n255\n", c.w, c.h).into_bytes();
        for p in &c.buf {
            out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, *p as u8]);
        }
        std::fs::write(path, out).unwrap();
    }
}
