//! screenctl — service téléphone : partage et pilotage de l'écran Android par câble USB.
//!
//! Incrément 1 : image fixe + toucher.
//!   - capture d'une trame via `screencap` (RGBA brut) ;
//!   - injection d'un toucher via `input tap` ;
//!   - transport TCP sur la seule interface de partage USB (RNDIS/NCM) ;
//!   - jeton aléatoire obligatoire, régénéré à chaque démarrage ;
//!   - journal des connexions pour le SOC.
//!
//! On tourne déjà en root (lancé par `su -c` depuis l'appli), donc `screencap` et
//! `input` sont appelés directement, sans `su` imbriqué.
//!
//! Protocole « PPSCR1 » (tout entier en gros-boutiste, ordre réseau) :
//!   Poignée de main  : le client envoie  b"PPSCR1 " + <jeton 32 hex> + b"\n".
//!   Puis, en boucle, un octet d'opcode :
//!     0x00 PING       -> réponse 1 octet 0x00
//!     0x01 GET_FRAME  -> réponse : u32 w, u32 h, u32 bpp, u32 len, [len octets RGBA]
//!     0x02 TAP        : u32 x, u32 y  -> réponse 1 octet 0x00
//!   Tout opcode inconnu ferme la connexion.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const PROTO: &[u8] = b"PPSCR1 ";
const DEFAULT_PORT: u16 = 8099;
const DEFAULT_LOG: &str = "/data/local/soc/screenctl.log";
/// Garde-fou : une trame d'écran brute dépasse rarement ~40 Mo (ex. 1080x2400x4 ≈ 10 Mo).
const MAX_FRAME: usize = 64 * 1024 * 1024;

const HELP: &str = "\
screenctl — partage/pilotage de l'écran Android par câble USB (root, PurpleOps)

Usage : screenctl serve [OPTIONS]

Options :
  --listen ADR:PORT   adresse d'écoute (défaut : <IP tether>:8099, sinon 0.0.0.0:8099)
  --iface NOM         lie la socket à cette seule interface (ex. rndis0, ncm0 ; root)
  --log FICHIER       journal des connexions (défaut /data/local/soc/screenctl.log)
  -h, --help          cette aide

À n'utiliser que sur son propre appareil, le temps d'une session (défaut : désactivé
dans l'appli). La socket doit rester liée au lien USB — jamais au Wi-Fi.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") || args.first().map(String::as_str) != Some("serve") {
        eprintln!("{HELP}");
        return if args.first().map(String::as_str) == Some("serve") { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }

    let listen = opt(&args, "--listen");
    let iface = opt(&args, "--iface");
    let log_path = opt(&args, "--log").unwrap_or_else(|| DEFAULT_LOG.to_string());

    match run(listen, iface, log_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("screenctl: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(listen: Option<String>, iface: Option<String>, log_path: String) -> io::Result<()> {
    // Adresse d'écoute : celle demandée, sinon l'IP de l'interface USB si on la trouve,
    // sinon 0.0.0.0 (mais on reste lié à l'interface via SO_BINDTODEVICE).
    let addr: SocketAddr = match &listen {
        Some(s) => s.parse().map_err(|_| err(format!("--listen invalide : {s}")))?,
        None => {
            let ip = iface.as_deref().and_then(iface_ipv4).unwrap_or(Ipv4Addr::UNSPECIFIED);
            SocketAddr::from((ip, DEFAULT_PORT))
        }
    };

    let token = hex(&random_bytes(16)?);
    let mut log = Logger::open(&log_path);

    let listener = bind(addr, iface.as_deref())?;
    let local = listener.local_addr()?;

    // Ligne que la section de l'appli récupère au regex pour afficher le jeton.
    println!("SCREENCTL token={token} listen={local} iface={}", iface.as_deref().unwrap_or("-"));
    println!("Jeton de session : {token}");
    println!("Écoute sur {local} (lien USB uniquement). En attente du PC…");
    let _ = io::stdout().flush();
    log.line(&format!("start listen={local} iface={}", iface.as_deref().unwrap_or("-")));

    for conn in listener.incoming() {
        let stream = match conn {
            Ok(s) => s,
            Err(e) => {
                log.line(&format!("accept error: {e}"));
                continue;
            }
        };
        let peer = stream.peer_addr().map(|p| p.to_string()).unwrap_or_else(|_| "?".into());
        // Incrément 1 : un client à la fois, en ligne (le flux vidéo multi-session
        // viendra avec l'incrément 2). On journalise entrée et sortie.
        log.line(&format!("conn from {peer}"));
        match serve_client(stream, &token, &mut log, &peer) {
            Ok(()) => log.line(&format!("close {peer} ok")),
            Err(e) => log.line(&format!("close {peer} err: {e}")),
        }
    }
    Ok(())
}

fn serve_client(mut s: TcpStream, token: &str, log: &mut Logger, peer: &str) -> io::Result<()> {
    s.set_nodelay(true).ok();

    // Poignée de main : b"PPSCR1 " + jeton + "\n".
    let mut head = vec![0u8; PROTO.len()];
    s.read_exact(&mut head)?;
    if head != PROTO {
        log.line(&format!("reject {peer}: mauvais protocole"));
        return Err(err("protocole inattendu".into()));
    }
    let mut got = Vec::with_capacity(token.len());
    let mut b = [0u8; 1];
    loop {
        s.read_exact(&mut b)?;
        if b[0] == b'\n' {
            break;
        }
        got.push(b[0]);
        if got.len() > token.len() {
            break; // trop long -> on va rejeter
        }
    }
    if !ct_eq(&got, token.as_bytes()) {
        log.line(&format!("reject {peer}: jeton invalide"));
        // réponse muette puis fermeture : on ne renseigne pas l'attaquant.
        return Err(err("jeton invalide".into()));
    }
    log.line(&format!("auth {peer} ok"));

    loop {
        let mut op = [0u8; 1];
        if s.read_exact(&mut op).is_err() {
            return Ok(()); // client parti
        }
        match op[0] {
            0x00 => {
                s.write_all(&[0x00])?;
            }
            0x01 => {
                let (w, h, bpp, pixels) = match capture_frame() {
                    Ok(f) => f,
                    Err(e) => {
                        log.line(&format!("screencap err: {e}"));
                        // 0,0,0,0 = pas de trame, le client réessaie.
                        s.write_all(&0u32.to_be_bytes())?;
                        s.write_all(&0u32.to_be_bytes())?;
                        s.write_all(&0u32.to_be_bytes())?;
                        s.write_all(&0u32.to_be_bytes())?;
                        continue;
                    }
                };
                s.write_all(&(w as u32).to_be_bytes())?;
                s.write_all(&(h as u32).to_be_bytes())?;
                s.write_all(&(bpp as u32).to_be_bytes())?;
                s.write_all(&(pixels.len() as u32).to_be_bytes())?;
                s.write_all(&pixels)?;
            }
            0x02 => {
                let mut xy = [0u8; 8];
                s.read_exact(&mut xy)?;
                let x = u32::from_be_bytes(xy[0..4].try_into().unwrap());
                let y = u32::from_be_bytes(xy[4..8].try_into().unwrap());
                tap(x, y);
                log.line(&format!("tap {peer} {x},{y}"));
                s.write_all(&[0x00])?;
            }
            other => {
                log.line(&format!("opcode inconnu {other:#x} de {peer}"));
                return Err(err(format!("opcode inconnu {other:#x}")));
            }
        }
        s.flush()?;
    }
}

/// Capture une trame via `screencap` (format brut). Renvoie (largeur, hauteur,
/// octets par pixel, pixels). On gère l'en-tête 12 ou 16 octets (champ colorspace
/// ajouté depuis Android 9).
fn capture_frame() -> io::Result<(usize, usize, usize, Vec<u8>)> {
    let out = Command::new("screencap").stdin(Stdio::null()).output()?;
    if !out.status.success() {
        return Err(err(format!("screencap code {:?}", out.status.code())));
    }
    let buf = out.stdout;
    if buf.len() < 12 {
        return Err(err("sortie screencap trop courte".into()));
    }
    let w = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    let h = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;
    let fmt = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    // Format 1 = RGBA_8888 (4 o/px) ; on ne traite que du 4 o/px pour l'instant.
    let bpp = 4usize;
    let need = w.checked_mul(h).and_then(|p| p.checked_mul(bpp)).ok_or_else(|| err("dimensions aberrantes".into()))?;
    if need == 0 || need > MAX_FRAME {
        return Err(err(format!("trame hors bornes : {w}x{h} fmt={fmt}")));
    }
    // En-tête 12 octets, ou 16 si colorspace présent.
    let header = if buf.len() >= 12 + need && buf.len() < 16 + need {
        12
    } else if buf.len() >= 16 + need {
        16
    } else {
        return Err(err(format!("taille screencap incohérente : {} octets pour {w}x{h}", buf.len())));
    };
    let pixels = buf[header..header + need].to_vec();
    Ok((w, h, bpp, pixels))
}

/// Injection d'un toucher (incrément 1 : simple et sans mise en place).
/// L'incrément suivant passera par /dev/uinput pour la latence et le multitouch.
fn tap(x: u32, y: u32) {
    let _ = Command::new("input")
        .args(["tap", &x.to_string(), &y.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

// ---------------------------------------------------------------------------
// Réseau : bind + éventuel SO_BINDTODEVICE (comme --iface de p900ctl).
// ---------------------------------------------------------------------------

fn bind(addr: SocketAddr, iface: Option<&str>) -> io::Result<TcpListener> {
    let listener = TcpListener::bind(addr)?;
    if let Some(name) = iface {
        bind_to_device(&listener, name)?;
    }
    Ok(listener)
}

#[cfg(target_os = "android")]
fn bind_to_device(listener: &TcpListener, iface: &str) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let fd = listener.as_raw_fd();
    // SO_BINDTODEVICE = 25 (SOL_SOCKET). Nécessite root ; on l'est.
    let mut name = iface.as_bytes().to_vec();
    name.push(0);
    let ret = unsafe {
        setsockopt(fd, 1 /*SOL_SOCKET*/, 25 /*SO_BINDTODEVICE*/, name.as_ptr().cast(), name.len() as u32)
    };
    if ret != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn bind_to_device(_listener: &TcpListener, _iface: &str) -> io::Result<()> {
    // Hors Android (tests sur l'hôte) : pas de SO_BINDTODEVICE, sans effet.
    Ok(())
}

#[cfg(target_os = "android")]
extern "C" {
    fn setsockopt(fd: i32, level: i32, optname: i32, optval: *const std::ffi::c_void, optlen: u32) -> i32;
}

/// Cherche l'adresse IPv4 d'une interface en lisant la sortie de `ip addr show <iface>`.
/// Best-effort : si absent, on retombe sur 0.0.0.0 (toujours lié via SO_BINDTODEVICE).
fn iface_ipv4(iface: &str) -> Option<Ipv4Addr> {
    let out = Command::new("ip").args(["-4", "addr", "show", iface]).stdin(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("inet ") {
            if let Some(cidr) = rest.split_whitespace().next() {
                if let Some(ip) = cidr.split('/').next() {
                    if let Ok(a) = ip.parse::<Ipv4Addr>() {
                        return Some(a);
                    }
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Journal SOC
// ---------------------------------------------------------------------------

struct Logger {
    file: Option<File>,
}

impl Logger {
    fn open(path: &str) -> Self {
        if let Some(dir) = PathBuf::from(path).parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = OpenOptions::new().create(true).append(true).open(path).ok();
        if file.is_none() {
            eprintln!("screenctl: journal indisponible ({path}), on continue sans");
        }
        Logger { file }
    }
    fn line(&mut self, msg: &str) {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let entry = format!("{ts} screenctl {msg}\n");
        if let Some(f) = self.file.as_mut() {
            let _ = f.write_all(entry.as_bytes());
            let _ = f.flush();
        }
        eprint!("{entry}");
    }
}

// ---------------------------------------------------------------------------
// Petits utilitaires (repris du style p900ctl)
// ---------------------------------------------------------------------------

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn random_bytes(n: usize) -> io::Result<Vec<u8>> {
    let mut v = vec![0u8; n];
    File::open("/dev/urandom")?.read_exact(&mut v)?;
    Ok(v)
}

/// Comparaison à temps constant pour le jeton.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn err(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg)
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
    fn hex_len() {
        assert_eq!(hex(&[0x0f, 0xa0]), "0fa0");
        assert_eq!(hex(&random_bytes(16).unwrap()).len(), 32);
    }

    #[test]
    fn opt_parsing() {
        let a: Vec<String> = ["serve", "--iface", "rndis0", "--listen", "1.2.3.4:9"].iter().map(|s| s.to_string()).collect();
        assert_eq!(opt(&a, "--iface").as_deref(), Some("rndis0"));
        assert_eq!(opt(&a, "--listen").as_deref(), Some("1.2.3.4:9"));
        assert_eq!(opt(&a, "--absent"), None);
    }
}
