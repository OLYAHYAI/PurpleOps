//! screenctl — service téléphone : partage et pilotage de l'écran Android par le câble USB.
//!
//! - Lien : prise USB en carte réseau NCM (`--usb`) ; écoute sur cette seule
//!   interface ; réarmement automatique quand on débranche / rebranche.
//! - Vidéo : H.264 matériel (`screenrecord`) en continu.
//! - Entrées : écran tactile et clavier virtuels (/dev/uinput) : instantané.
//! - Accès : appairage façon ADB. Le PC présente une clé qu'il garde ; une clé
//!   inconnue n'est acceptée que pendant la fenêtre d'appairage ouverte depuis
//!   l'appli (bouton APPAIRER, 2 min). Liste : `<dir>/screenctl_pcs`.
//! - Journal SOC : `<dir>/screenctl.log` (ingéré par l'agent Wazuh).
//!
//! - Presse-papiers et texte Unicode : aide Java de l'APK lancée par `app_process`
//!   (`ScreenHelper.kt`, uid shell), pilotée par stdin/stdout.
//!
//! Protocole « PPSCR3 » (entiers gros-boutistes) :
//!   C->S  "PPSCR3 <clé 64 hex> <nom du PC>\n"
//!   S->C  1 octet : 0x00 autorisé | 0x02 refusé (PC non appairé)
//!   C->S  0x05 STREAM u32 débit (bit/s, 0 = défaut)
//!   S->C  u32 largeur, u32 hauteur de l'écran, puis des messages :
//!         u8 type + u32 taille + contenu
//!           1 = image H.264 Annex-B complète (une unité d'accès)
//!           2 = presse-papiers du téléphone (UTF-8)
//!   C->S  événements, sans réponse :
//!         0x10 DOWN x y | 0x11 MOVE x y | 0x12 UP
//!         0x20 KEY code(Linux) mods(bit0 Maj, bit1 Ctrl, bit2 Alt)
//!         0x21 TEXT len + UTF-8   | 0x22 CLIP len + UTF-8 (presse-papiers du PC)
//!         0x30 SCROLL x y dy(i32) | 0x00 PING

mod uinput;

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROTO: &str = "PPSCR3";
const DEFAULT_PORT: u16 = 8099;
const DEFAULT_DIR: &str = "/data/local/soc";
const DEFAULT_BITRATE: u32 = 16_000_000;

const HELP: &str = "\
screenctl — écran Android piloté depuis le PC par le câble USB (root, PurpleOps)

Usage : screenctl serve [--usb] [--iface ncm0] [--port 8099] [--dir /data/local/soc]
        screenctl pair [SECONDES]   ouvre la fenêtre d'appairage (défaut 120 s)
        screenctl forget            oublie tous les PC appairés
        screenctl usb-off           remet la prise USB en mode normal

  --usb     passe la prise USB en carte réseau (NCM) et la réarme si on rebranche
  --listen  ADR:PORT fixe (tests) au lieu de suivre l'interface";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(opt(&args, "--dir").unwrap_or_else(|| DEFAULT_DIR.into()));
    let res = match args.first().map(String::as_str) {
        Some("usb-off") => {
            usb_functions("");
            say("Prise USB remise en mode normal.");
            Ok(())
        }
        Some("pair") => {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(120);
            let _ = fs::create_dir_all(&dir);
            fs::write(dir.join("screenctl.pairing"), (now() + secs).to_string())
                .map(|_| say(&format!("Appairage ouvert {secs} s : lance screenview.exe sur le PC.")))
        }
        Some("forget") => {
            let _ = fs::remove_file(dir.join("screenctl_pcs"));
            say("Tous les PC sont oubliés.");
            Ok(())
        }
        Some("serve") if !args.iter().any(|a| a == "-h" || a == "--help") => serve(&args, dir),
        _ => {
            eprintln!("{HELP}");
            return ExitCode::FAILURE;
        }
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            say(&format!("screenctl: {e}"));
            ExitCode::FAILURE
        }
    }
}

fn serve(args: &[String], dir: PathBuf) -> io::Result<()> {
    let usb = args.iter().any(|a| a == "--usb");
    let iface = opt(args, "--iface").or_else(|| usb.then(|| "ncm0".into()));
    let port: u16 = opt(args, "--port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let fixed: Option<SocketAddr> = match opt(args, "--listen") {
        Some(s) => Some(s.parse().map_err(|_| err(format!("--listen invalide : {s}")))?),
        None => None,
    };
    if fixed.is_none() && iface.is_none() {
        return Err(err("indiquer --usb, --iface ou --listen".into()));
    }
    let _ = fs::create_dir_all(&dir);
    let log = Arc::new(Mutex::new(Logger::open(&dir.join("screenctl.log"))));
    let ctx = Ctx { dir, log };
    ctx.log(&format!("start iface={} usb={usb}", iface.as_deref().unwrap_or("-")));
    leave_app_cgroup();

    loop {
        let addr = match (fixed, iface.as_deref()) {
            (Some(a), _) => a,
            (None, Some(name)) => SocketAddr::from((wait_link(name, usb), port)),
            _ => unreachable!(),
        };
        let listener = match TcpListener::bind(addr) {
            Ok(l) => l,
            Err(e) => {
                ctx.log(&format!("bind {addr} : {e}"));
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        if let Some(name) = iface.as_deref() {
            if let Err(e) = bind_to_device(&listener, name) {
                ctx.log(&format!("SO_BINDTODEVICE {name} : {e}"));
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        }
        listener.set_nonblocking(true)?;
        say(&format!("SCREENCTL listen={addr} iface={}", iface.as_deref().unwrap_or("-")));
        ctx.log(&format!("listen {addr}"));

        let mut last_check = Instant::now();
        loop {
            match listener.accept() {
                Ok((s, peer)) => {
                    let _ = s.set_nonblocking(false);
                    let peer = peer.to_string();
                    if let Err(e) = ctx.session(s, &peer) {
                        ctx.log(&format!("session {peer} : {e}"));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(200)),
                Err(e) => ctx.log(&format!("accept : {e}")),
            }
            // Câble débranché / rebranché : Android peut changer d'adresse -> on se réarme.
            if fixed.is_none() && last_check.elapsed() > Duration::from_secs(2) {
                last_check = Instant::now();
                if iface.as_deref().and_then(iface_ipv4) != Some(*match &addr {
                    SocketAddr::V4(a) => a.ip(),
                    _ => unreachable!(),
                }) {
                    say("SCREENCTL link-lost");
                    ctx.log("lien USB perdu, réarmement");
                    break;
                }
            }
        }
    }
}

/// Lancé par `su` depuis l'appli, le service hérite du groupe de processus
/// (cgroup) de l'appli : Android le tuerait avec elle. On passe dans le groupe
/// racine pour pouvoir continuer à tourner appli fermée (démarrage automatique).
fn leave_app_cgroup() {
    let pid = std::process::id().to_string();
    for p in ["/sys/fs/cgroup/cgroup.procs", "/acct/cgroup.procs"] {
        let _ = fs::write(p, &pid);
    }
}

/// Attend que l'interface ait une adresse. Avec --usb, (re)passe la prise en NCM
/// tant que l'interface n'existe pas : après un débranchement Android revient au
/// mode USB par défaut.
fn wait_link(iface: &str, usb: bool) -> Ipv4Addr {
    let mut said = false;
    let mut last_set: Option<Instant> = None;
    loop {
        if let Some(ip) = iface_ipv4(iface) {
            say(&format!("Lien USB actif : {iface} = {ip}"));
            return ip;
        }
        if !said {
            say("SCREENCTL waiting-cable");
            said = true;
        }
        if usb && last_set.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
            usb_functions("ncm");
            last_set = Some(Instant::now());
        }
        thread::sleep(Duration::from_millis(500));
    }
}

struct Ctx {
    dir: PathBuf,
    log: Arc<Mutex<Logger>>,
}

impl Ctx {
    fn log(&self, msg: &str) {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).line(msg);
    }

    fn session(&self, mut s: TcpStream, peer: &str) -> io::Result<()> {
        s.set_nodelay(true).ok();
        s.set_read_timeout(Some(Duration::from_secs(10)))?;
        let line = read_line(&mut s, 200)?;
        let mut it = line.splitn(3, ' ');
        let (proto, key, host) = (it.next().unwrap_or(""), it.next().unwrap_or(""), it.next().unwrap_or("?"));
        let host: String = host.chars().filter(|c| c.is_ascii_alphanumeric() || "-_.".contains(*c)).take(40).collect();
        if proto != PROTO || key.len() != 64 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
            self.log(&format!("reject {peer}: poignée de main invalide"));
            return Ok(());
        }
        let key = key.to_ascii_lowercase();

        if self.is_paired(&key) {
            self.log(&format!("auth {peer} ok host={host}"));
        } else if self.pairing_open() {
            self.add_pc(&key, &host)?;
            let _ = fs::remove_file(self.dir.join("screenctl.pairing"));
            self.log(&format!("pair {peer} host={host}"));
            say(&format!("SCREENCTL paired host={host}"));
        } else {
            self.log(&format!("refuse {peer} host={host}: PC non appairé"));
            say(&format!("SCREENCTL refused host={host}"));
            s.write_all(&[0x02])?;
            return Ok(());
        }
        s.write_all(&[0x00])?;
        say(&format!("SCREENCTL connected host={host}"));

        let mut op = [0u8; 1];
        s.read_exact(&mut op)?;
        if op[0] != 0x05 {
            return Err(err(format!("opcode inattendu {:#x}", op[0])));
        }
        let b = read_u32s::<1>(&mut s)?[0];
        let bitrate = if b == 0 { DEFAULT_BITRATE } else { b.clamp(500_000, 40_000_000) };
        s.set_read_timeout(None)?;
        let r = self.stream(s, bitrate, peer);
        say(&format!("SCREENCTL disconnected host={host}"));
        r
    }

    fn is_paired(&self, key: &str) -> bool {
        let Ok(txt) = fs::read_to_string(self.dir.join("screenctl_pcs")) else { return false };
        txt.lines().filter_map(|l| l.split_whitespace().next()).any(|k| ct_eq(k.as_bytes(), key.as_bytes()))
    }

    fn pairing_open(&self) -> bool {
        fs::read_to_string(self.dir.join("screenctl.pairing"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .is_some_and(|exp| now() <= exp)
    }

    fn add_pc(&self, key: &str, host: &str) -> io::Result<()> {
        let path = self.dir.join("screenctl_pcs");
        let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(f, "{key} {host} {}", now())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// Session : vidéo dans un fil, événements d'entrée dans celui-ci.
    fn stream(&self, s: TcpStream, bitrate: u32, peer: &str) -> io::Result<()> {
        let (w, h) = display_size();
        let mut first = s.try_clone()?;
        first.write_all(&w.to_be_bytes())?;
        first.write_all(&h.to_be_bytes())?;
        let out: Out = Arc::new(Mutex::new(first));
        self.log(&format!("stream {peer} {w}x{h} h264 {bitrate} bit/s"));
        let helper = Helper::start(out.clone(), self.log.clone());

        let mut touch = uinput::Touch::new(w, h).map_err(|e| self.log(&format!("uinput tactile : {e}"))).ok();
        let mut kbd = uinput::Keyboard::new().map_err(|e| self.log(&format!("uinput clavier : {e}"))).ok();

        let alive = Arc::new(AtomicBool::new(true));
        let child: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let video = spawn_video(out.clone(), bitrate, alive.clone(), child.clone());

        let mut s = s;
        let mut skipped = 0u32;
        let mut fallback_down: Option<(u32, u32)> = None;
        let res: io::Result<()> = (|| loop {
            if !alive.load(Ordering::SeqCst) {
                return Ok(());
            }
            let mut op = [0u8; 1];
            if s.read_exact(&mut op).is_err() {
                return Ok(());
            }
            match op[0] {
                0x00 => {}
                0x10 | 0x11 => {
                    let v = read_u32s::<2>(&mut s)?;
                    let (x, y) = (v[0].min(w - 1), v[1].min(h - 1));
                    match touch.as_mut() {
                        Some(t) if op[0] == 0x10 => t.down(x, y)?,
                        Some(t) => t.moved(x, y)?,
                        None if op[0] == 0x10 => fallback_down = Some((x, y)),
                        None => {}
                    }
                }
                0x12 => match touch.as_mut() {
                    Some(t) => t.up()?,
                    None => {
                        if let Some((x, y)) = fallback_down.take() {
                            input(&["tap", &x.to_string(), &y.to_string()]);
                        }
                    }
                },
                0x20 => {
                    let v = read_u32s::<2>(&mut s)?;
                    if let Some(k) = kbd.as_mut() {
                        k.tap(v[0] as u16, v[1])?;
                    }
                }
                0x21 => {
                    let len = read_u32s::<1>(&mut s)?[0] as usize;
                    if len > 64 * 1024 {
                        return Err(err("texte trop long".into()));
                    }
                    let mut b = vec![0u8; len];
                    s.read_exact(&mut b)?;
                    // Aide Java prête : texte Unicode, indépendant de la disposition.
                    if helper.send('T', &b) {
                        continue;
                    }
                    if let Some(k) = kbd.as_mut() {
                        for c in String::from_utf8_lossy(&b).chars() {
                            match uinput::us_key(c) {
                                Some((code, shift)) => k.tap(code, shift as u32)?,
                                None => skipped += 1,
                            }
                        }
                    }
                }
                0x22 => {
                    let len = read_u32s::<1>(&mut s)?[0] as usize;
                    if len > 1 << 20 {
                        return Err(err("presse-papiers trop long".into()));
                    }
                    let mut b = vec![0u8; len];
                    s.read_exact(&mut b)?;
                    helper.send('C', &b);
                }
                0x30 => {
                    let v = read_u32s::<3>(&mut s)?;
                    let (x, y, dy) = (v[0].min(w - 1) as i64, v[1].min(h - 1) as i64, v[2] as i32 as i64);
                    if let Some(t) = touch.as_mut() {
                        // Molette : petit glissé du doigt virtuel à l'endroit du pointeur.
                        t.down(x as u32, y as u32)?;
                        for i in 1..=6 {
                            thread::sleep(Duration::from_millis(8));
                            let yy = (y + dy * i / 6).clamp(0, h as i64 - 1);
                            t.moved(x as u32, yy as u32)?;
                        }
                        thread::sleep(Duration::from_millis(20));
                        t.up()?;
                    }
                }
                other => return Err(err(format!("opcode inconnu {other:#x}"))),
            }
        })();

        alive.store(false, Ordering::SeqCst);
        if let Some(mut c) = child.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = s.shutdown(std::net::Shutdown::Both);
        drop(touch);
        drop(kbd);
        helper.stop();
        let why = if video.is_finished() {
            video.join().unwrap_or_else(|_| "fil vidéo en erreur".into())
        } else {
            "client parti".into()
        };
        if skipped > 0 {
            self.log(&format!("{skipped} caractères non ASCII ignorés"));
        }
        self.log(&format!("stream {peer} fin : {why}"));
        res
    }
}

/// Fil vidéo : sortie H.264 de `screenrecord`, relancé à chaque fin de process
/// (limite de 3 min par appel), envoyée **image par image** : u32 taille + octets.
///
/// `screenrecord` écrit chaque image d'un seul appel système. On agrandit le tube
/// à 1 Mo pour que cette écriture ne bloque jamais, puis on lit tant que le tube
/// se remplit encore (attente de 3 ms au plus) : on obtient l'image entière, que le
/// PC décode sans jamais deviner où elle se termine.
fn spawn_video(
    out: Out,
    bitrate: u32,
    alive: Arc<AtomicBool>,
    child: Arc<Mutex<Option<std::process::Child>>>,
) -> thread::JoinHandle<String> {
    thread::spawn(move || -> String {
        let mut buf = vec![0u8; 1 << 20];
        let mut frame: Vec<u8> = Vec::with_capacity(1 << 20);
        let mut quick_fails = 0;
        while alive.load(Ordering::SeqCst) {
            let started = Instant::now();
            let spawned = Command::new("screenrecord")
                .args(["--output-format=h264", &format!("--bit-rate={bitrate}"), "--time-limit=180", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn();
            let mut c = match spawned {
                Ok(c) => c,
                Err(e) => return format!("screenrecord introuvable : {e}"),
            };
            let mut so = c.stdout.take().expect("stdout piped");
            pipe_grow(&so);
            *child.lock().unwrap_or_else(|p| p.into_inner()) = Some(c);
            'proc: loop {
                frame.clear();
                match so.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => frame.extend_from_slice(&buf[..n]),
                }
                while frame.len() < 16 << 20 && pipe_ready(&so, 3) {
                    match so.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => frame.extend_from_slice(&buf[..n]),
                    }
                }
                let ok = send_msg(&out, 1, &frame);
                if ok.is_err() {
                    alive.store(false, Ordering::SeqCst);
                    break 'proc;
                }
            }
            if let Some(mut c) = child.lock().unwrap_or_else(|p| p.into_inner()).take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            if started.elapsed() < Duration::from_secs(2) {
                quick_fails += 1;
                if quick_fails >= 5 {
                    alive.store(false, Ordering::SeqCst);
                    let _ = out.lock().unwrap_or_else(|p| p.into_inner()).shutdown(std::net::Shutdown::Both);
                    return "screenrecord échoue au démarrage (5 fois)".into();
                }
                thread::sleep(Duration::from_millis(300));
            } else {
                quick_fails = 0;
            }
        }
        "flux terminé".into()
    })
}

#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}

/// Des données attendent-elles dans le tube (dans la limite de `ms`) ?
fn pipe_ready(p: &std::process::ChildStdout, ms: i32) -> bool {
    use std::os::unix::io::AsRawFd;
    let mut fd = PollFd { fd: p.as_raw_fd(), events: 0x1, revents: 0 }; // POLLIN
    unsafe { poll(&mut fd, 1, ms) > 0 && fd.revents & 0x1 != 0 }
}

/// Tube de 1 Mo (F_SETPIPE_SZ) : une image entière y tient, l'écriture ne bloque pas.
fn pipe_grow(p: &std::process::ChildStdout) {
    use std::os::unix::io::AsRawFd;
    unsafe {
        fcntl(p.as_raw_fd(), 1031, 1 << 20);
    }
}

/// Sortie vers le PC, partagée entre le fil vidéo et le presse-papiers.
type Out = Arc<Mutex<TcpStream>>;

/// Message serveur -> PC : u8 type + u32 taille + contenu, écrit d'un bloc.
fn send_msg(out: &Out, typ: u8, data: &[u8]) -> io::Result<()> {
    let mut o = out.lock().unwrap_or_else(|p| p.into_inner());
    let mut head = [0u8; 5];
    head[0] = typ;
    head[1..].copy_from_slice(&(data.len() as u32).to_be_bytes());
    o.write_all(&head)?;
    o.write_all(data)
}

/// Aide Java (`ScreenHelper` dans l'APK) : presse-papiers et texte Unicode.
/// Lancée sous l'uid shell (comme scrcpy : accès presse-papiers et injection
/// autorisés), en root si cela échoue. Démarrage d'une JVM : ~1 s, en arrière-plan ;
/// en attendant, le texte passe par le clavier virtuel.
struct Helper {
    stdin: Arc<Mutex<Option<std::process::ChildStdin>>>,
    child: Arc<Mutex<Option<std::process::Child>>>,
}

impl Helper {
    fn start(out: Out, log: Arc<Mutex<Logger>>) -> Helper {
        let h = Helper { stdin: Arc::new(Mutex::new(None)), child: Arc::new(Mutex::new(None)) };
        let (stdin, child) = (h.stdin.clone(), h.child.clone());
        thread::spawn(move || {
            let logl = |m: &str| log.lock().unwrap_or_else(|p| p.into_inner()).line(m);
            let Some(apk) = apk_path() else {
                logl("aide : APK introuvable, presse-papiers désactivé");
                return;
            };
            for as_shell in [true, false] {
                let mut cmd = Command::new("app_process");
                cmd.env("CLASSPATH", &apk)
                    .args(["/system/bin", "com.jbr.purpleops.ScreenHelper"])
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                if as_shell {
                    use std::os::unix::process::CommandExt;
                    cmd.uid(2000).gid(2000);
                }
                let Ok(mut c) = cmd.spawn() else { continue };
                let (Some(si), Some(so)) = (c.stdin.take(), c.stdout.take()) else { continue };
                *child.lock().unwrap_or_else(|p| p.into_inner()) = Some(c);
                let mut lines = io::BufReader::new(so).lines();
                // Attend « R » : l'aide est prête.
                let ready = loop {
                    match lines.next() {
                        Some(Ok(l)) if l == "R" => break true,
                        Some(Ok(l)) if l.starts_with("E ") => logl(&format!("aide : {}", &l[2..])),
                        Some(Ok(_)) => {}
                        _ => break false,
                    }
                };
                if !ready {
                    if let Some(mut c) = child.lock().unwrap_or_else(|p| p.into_inner()).take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                    logl(&format!("aide : échec du démarrage ({})", if as_shell { "uid shell" } else { "root" }));
                    continue;
                }
                logl(&format!("aide prête ({})", if as_shell { "uid shell" } else { "root" }));
                *stdin.lock().unwrap_or_else(|p| p.into_inner()) = Some(si);
                for l in lines.map_while(Result::ok) {
                    if let Some(b) = l.strip_prefix("C ") {
                        if let Some(text) = b64_decode(b) {
                            if send_msg(&out, 2, &text).is_err() {
                                break;
                            }
                        }
                    } else if let Some(m) = l.strip_prefix("E ") {
                        logl(&format!("aide : {m}"));
                    }
                }
                return;
            }
        });
        h
    }

    /// Envoie une commande à l'aide ; faux si elle n'est pas (encore) prête.
    fn send(&self, cmd: char, data: &[u8]) -> bool {
        let mut g = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
        let Some(si) = g.as_mut() else { return false };
        let line = format!("{cmd} {}\n", b64_encode(data));
        if si.write_all(line.as_bytes()).and_then(|_| si.flush()).is_err() {
            *g = None;
            return false;
        }
        true
    }

    fn stop(&self) {
        self.stdin.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(mut c) = self.child.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// .../com.jbr.purpleops-XXX/lib/arm64/libscreenctl.so -> .../base.apk
fn apk_path() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let apk = exe.parent()?.parent()?.parent()?.join("base.apk");
    apk.exists().then(|| apk.to_string_lossy().into_owned())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(d: &[u8]) -> String {
    let mut s = String::with_capacity(d.len().div_ceil(3) * 4);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes().filter(|c| *c != b'=') {
        let v = B64.iter().position(|b| *b == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Taille de l'écran (taille forcée si définie) via `wm size`.
fn display_size() -> (u32, u32) {
    let out = Command::new("wm").arg("size").stdin(Stdio::null()).output();
    let txt = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let parse = |prefix: &str| {
        txt.lines().find_map(|l| {
            let (w, h) = l.strip_prefix(prefix)?.trim().split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
    };
    parse("Override size:").or_else(|| parse("Physical size:")).unwrap_or((1080, 2400))
}

fn read_line(s: &mut TcpStream, max: usize) -> io::Result<String> {
    let mut v = Vec::new();
    let mut b = [0u8; 1];
    loop {
        s.read_exact(&mut b)?;
        if b[0] == b'\n' {
            break;
        }
        v.push(b[0]);
        if v.len() > max {
            return Err(err("ligne trop longue".into()));
        }
    }
    Ok(String::from_utf8_lossy(&v).trim().to_string())
}

fn read_u32s<const N: usize>(s: &mut TcpStream) -> io::Result<[u32; N]> {
    let mut out = [0u32; N];
    for v in out.iter_mut() {
        let mut b = [0u8; 4];
        s.read_exact(&mut b)?;
        *v = u32::from_be_bytes(b);
    }
    Ok(out)
}

/// Repli si uinput est indisponible : commande `input` (lente).
fn input(args: &[&str]) {
    let _ = Command::new("input").args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn usb_functions(f: &str) {
    let _ = Command::new("svc")
        .args(["usb", "setFunctions", f])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(target_os = "android")]
fn bind_to_device(listener: &TcpListener, iface: &str) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let mut name = iface.as_bytes().to_vec();
    name.push(0);
    // SOL_SOCKET = 1, SO_BINDTODEVICE = 25
    let ret = unsafe { setsockopt(listener.as_raw_fd(), 1, 25, name.as_ptr().cast(), name.len() as u32) };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn bind_to_device(_listener: &TcpListener, _iface: &str) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "android")]
extern "C" {
    fn setsockopt(fd: i32, level: i32, optname: i32, optval: *const std::ffi::c_void, optlen: u32) -> i32;
}

/// Adresse IPv4 d'une interface (sortie de `ip -4 addr show`).
fn iface_ipv4(iface: &str) -> Option<Ipv4Addr> {
    let out = Command::new("ip").args(["-4", "addr", "show", iface]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|l| {
        let cidr = l.trim().strip_prefix("inet ")?.split_whitespace().next()?;
        cidr.split('/').next()?.parse().ok()
    })
}

/// Sortie standard sans jamais paniquer : si l'appli qui lisait notre sortie a été
/// fermée, le service doit continuer à tourner.
fn say(msg: &str) {
    let mut o = io::stdout();
    let _ = writeln!(o, "{msg}");
    let _ = o.flush();
}

struct Logger {
    file: Option<File>,
}

impl Logger {
    fn open(path: &Path) -> Self {
        Logger { file: OpenOptions::new().create(true).append(true).open(path).ok() }
    }
    fn line(&mut self, msg: &str) {
        let entry = format!("{} screenctl {msg}\n", now());
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(entry.as_bytes());
        }
        let _ = io::stderr().write_all(entry.as_bytes());
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

/// Comparaison à temps constant.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn err(msg: String) -> io::Error {
    io::Error::other(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_basic() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn base64_roundtrip() {
        for s in ["", "a", "ab", "abc", "été 👍 «ok»"] {
            let e = b64_encode(s.as_bytes());
            assert_eq!(b64_decode(&e).unwrap(), s.as_bytes());
        }
        assert_eq!(b64_encode(b"Man"), "TWFu");
        assert_eq!(b64_encode(b"Ma"), "TWE=");
    }

    #[test]
    fn opt_parsing() {
        let a: Vec<String> = ["serve", "--iface", "ncm0", "--port", "9"].iter().map(|s| s.to_string()).collect();
        assert_eq!(opt(&a, "--iface").as_deref(), Some("ncm0"));
        assert_eq!(opt(&a, "--port").as_deref(), Some("9"));
        assert_eq!(opt(&a, "--absent"), None);
    }
}
