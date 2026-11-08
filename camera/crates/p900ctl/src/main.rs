//! p900ctl : pilote une Nikon COOLPIX P900 (et COOLPIX Wi-Fi proches) en PTP/IP.

mod web;

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ptpip::codes::*;
use ptpip::dataset::{self, Form, PropDesc, Value};
use ptpip::{Camera, DataSink, Options};

const HELP: &str = "\
p900ctl — pilote une Nikon COOLPIX P900 en Wi-Fi (PTP/IP, port 15740)

Usage : p900ctl [OPTIONS] COMMANDE [ARGUMENTS]

Commandes (lecture seule) :
  probe [--save FICHIER]      connexion, puis liste de ce que l'appareil accepte
  props [--save FICHIER]      réglages exposés : type, valeur, plage, modifiable ou non
  ls [--last N]               supports et photos de la carte
  watch [--seconds N]         affiche les événements et changements de réglages
  guid                        affiche l'identifiant d'initiateur utilisé

Commandes d'action :
  get HANDLE... | --all | --last N [--thumb] [--out DOSSIER] [--force]
  capture [--af] [--no-download] [--out DOSSIER]
  liveview [--frames N] [--out DOSSIER]   mesure la cadence de visée
  zoom in|out|VALEUR          zoom par la propriété FocalLength, si modifiable
  set-prop CODE VALEUR [--force]
  serve [--listen ADR:PORT] [--out DOSSIER]   visée + boutons dans le navigateur

Options :
  --host IP        adresse de l'appareil (défaut : passerelle Wi-Fi, puis 192.168.0.10…)
  --port N         port PTP/IP (défaut 15740)
  --iface NOM      lie les sockets à cette interface (ex. wlan0, root requis)
  --guid HEX32     identifiant d'initiateur (défaut : aléatoire, conservé)
  --name NOM       nom présenté à l'appareil (défaut p900ctl)
  --timeout S      délai de réponse en secondes (défaut 10)
  --show-serial    affiche le numéro de série en clair
  --trace          journalise les paquets PTP/IP sur la sortie d'erreur
";

const VALUE_FLAGS: &[&str] = &[
    "--host", "--port", "--iface", "--guid", "--name", "--timeout", "--out", "--save",
    "--seconds", "--frames", "--listen", "--last",
];
const SWITCHES: &[&str] = &[
    "--trace", "--show-serial", "--af", "--force", "--all", "--thumb", "--no-download",
    "--help", "-h", "--version", "-V",
];

struct Args {
    pos: Vec<String>,
    vals: HashMap<String, String>,
    sw: HashSet<String>,
}

impl Args {
    fn parse() -> Result<Args, String> {
        let mut a = Args { pos: vec![], vals: HashMap::new(), sw: HashSet::new() };
        let mut it = std::env::args().skip(1);
        while let Some(x) = it.next() {
            if VALUE_FLAGS.contains(&x.as_str()) {
                let v = it.next().ok_or_else(|| format!("{x} attend une valeur"))?;
                a.vals.insert(x, v);
            } else if SWITCHES.contains(&x.as_str()) {
                a.sw.insert(x);
            } else if x.starts_with("--") {
                return Err(format!("option inconnue : {x}"));
            } else {
                a.pos.push(x);
            }
        }
        Ok(a)
    }
    fn val(&self, k: &str) -> Option<&str> {
        self.vals.get(k).map(|s| s.as_str())
    }
    fn has(&self, k: &str) -> bool {
        self.sw.contains(k)
    }
    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> Result<T, String> {
        match self.val(k) {
            Some(v) => v.parse().map_err(|_| format!("{k} : valeur invalide « {v} »")),
            None => Ok(default),
        }
    }
}

type Res<T> = Result<T, String>;

fn e<E: std::fmt::Display>(x: E) -> String {
    x.to_string()
}

fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(a) => a,
        Err(m) => {
            eprintln!("{m}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    if args.has("--help") || args.has("-h") || args.pos.is_empty() {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args.has("--version") || args.has("-V") {
        println!("p900ctl {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(m) => {
            eprintln!("erreur : {m}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Res<()> {
    if args.has("--trace") {
        ptpip::packet::TRACE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let cmd = args.pos[0].as_str();
    let (guid, guid_path) = match args.val("--guid") {
        Some(h) => (parse_guid(h)?, None),
        None => {
            let (g, p) = load_or_create_guid().map_err(|x| format!("GUID : {x}"))?;
            (g, Some(p))
        }
    };
    if cmd == "guid" {
        println!("{}", hex(&guid));
        if let Some(p) = guid_path {
            println!("conservé dans {}", p.display());
        }
        return Ok(());
    }
    let opts = Options {
        host: match args.val("--host") {
            Some(h) => Some(h.parse::<Ipv4Addr>().map_err(|_| format!("--host : adresse IPv4 invalide « {h} »"))?),
            None => None,
        },
        port: args.num("--port", 15740u16)?,
        iface: args.val("--iface").map(String::from),
        guid,
        name: args.val("--name").unwrap_or("p900ctl").to_string(),
        timeout: Duration::from_secs(args.num("--timeout", 10u64)?),
    };

    let known = [
        "probe", "props", "ls", "watch", "get", "capture", "liveview", "zoom", "set-prop", "serve",
    ];
    if !known.contains(&cmd) {
        return Err(format!("commande inconnue : {cmd} (voir p900ctl --help)"));
    }

    eprintln!(
        "Connexion{}{}…",
        opts.host.map(|h| format!(" à {h}:{}", opts.port)).unwrap_or_default(),
        opts.iface.as_ref().map(|i| format!(" par {i}")).unwrap_or_default()
    );
    let t0 = Instant::now();
    let mut cam = Camera::connect(&opts).map_err(e)?;
    eprintln!(
        "Poignée de main PTP/IP : OK en {} ms — {} répond « {} » (connexion n° {})",
        t0.elapsed().as_millis(),
        cam.addr,
        cam.peer_name,
        cam.conn_number
    );
    cam.open_session().map_err(e)?;
    cam.device_info().map_err(e)?;

    match cmd {
        "probe" => cmd_probe(&mut cam, args),
        "props" => cmd_props(&mut cam, args),
        "ls" => cmd_ls(&mut cam, args),
        "watch" => cmd_watch(&mut cam, args),
        "get" => cmd_get(&mut cam, args),
        "capture" => cmd_capture(&mut cam, args),
        "liveview" => cmd_liveview(&mut cam, args),
        "zoom" => cmd_zoom(&mut cam, args),
        "set-prop" => cmd_set_prop(&mut cam, args),
        "serve" => {
            let listen: SocketAddr = args
                .val("--listen")
                .unwrap_or("127.0.0.1:8080")
                .parse()
                .map_err(|_| "--listen : adresse invalide".to_string())?;
            web::serve(cam, listen, out_dir(args)?)
        }
        _ => unreachable!(),
    }
}

// --- Utilitaires -----------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn parse_guid(h: &str) -> Res<[u8; 16]> {
    let h: String = h.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if h.len() != 32 {
        return Err("--guid attend 32 chiffres hexadécimaux".into());
    }
    let mut g = [0u8; 16];
    for (i, b) in g.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).map_err(e)?;
    }
    Ok(g)
}

fn config_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(x).join("p900ctl");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".config").join("p900ctl")
}

pub(crate) fn random_bytes(n: usize) -> io::Result<Vec<u8>> {
    use std::io::Read;
    let mut v = vec![0u8; n];
    File::open("/dev/urandom")?.read_exact(&mut v)?;
    Ok(v)
}

fn load_or_create_guid() -> io::Result<([u8; 16], PathBuf)> {
    let dir = config_dir();
    let path = dir.join("guid");
    if let Ok(s) = fs::read_to_string(&path) {
        if let Ok(g) = parse_guid(s.trim()) {
            return Ok((g, path));
        }
    }
    fs::create_dir_all(&dir)?;
    let mut g = [0u8; 16];
    g.copy_from_slice(&random_bytes(16)?);
    let mut o = OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    writeln!(o.open(&path)?, "{}", hex(&g))?;
    Ok((g, path))
}

fn out_dir(args: &Args) -> Res<PathBuf> {
    let d = PathBuf::from(args.val("--out").unwrap_or("."));
    fs::create_dir_all(&d).map_err(|x| format!("{} : {x}", d.display()))?;
    Ok(d)
}

/// Écrit à l'écran et, avec --save, dans un fichier.
struct Report {
    file: Option<BufWriter<File>>,
}

impl Report {
    fn new(args: &Args) -> Res<Report> {
        let file = match args.val("--save") {
            Some(p) => Some(BufWriter::new(File::create(p).map_err(|x| format!("{p} : {x}"))?)),
            None => None,
        };
        Ok(Report { file })
    }
    fn line(&mut self, s: impl AsRef<str>) {
        // Ignore une sortie fermée (ex. `| head`) au lieu de paniquer.
        let _ = writeln!(io::stdout(), "{}", s.as_ref());
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{}", s.as_ref());
        }
    }
}

fn op_label(c: u16) -> String {
    format!("{c:#06x}  {}", op_name(c).unwrap_or("(inconnue)"))
}

fn mask_serial(s: &str, show: bool) -> String {
    if show || s.len() <= 4 {
        return s.to_string();
    }
    let tail: String = s.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("****{tail}")
}

const FEATURES: &[(&str, &[u16])] = &[
    ("Visée à distance (Nikon)", &[0x9201, 0x9202, 0x9203]),
    ("Attente « appareil prêt » (Nikon)", &[0x90C8]),
    ("Déclenchement vers la carte (Nikon)", &[0x9207]),
    ("Déclenchement PTP standard", &[0x100E]),
    ("Événements Nikon (CheckEvent)", &[0x90C7]),
    ("Liste et téléchargement des photos", &[0x1007, 0x1008, 0x1009]),
    ("Vignettes", &[0x100A]),
    ("Lecture des réglages", &[0x1014, 0x1015]),
    ("Modification des réglages", &[0x1016]),
    ("Autofocus à la demande (Nikon)", &[0x90C1]),
    ("Mise au point manuelle (Nikon)", &[0x9204]),
    ("Vidéo sur la carte (Nikon)", &[0x920A, 0x920B]),
    ("Réglages cachés Nikon", &[0x90CA]),
    ("Suppression de photos", &[0x100B]),
];

// --- probe -------------------------------------------------------------------------

fn cmd_probe(cam: &mut Camera, args: &Args) -> Res<()> {
    let mut r = Report::new(args)?;
    let info = cam.info.clone().unwrap_or_default();
    r.line(format!("p900ctl {} — relevé PTP/IP", env!("CARGO_PKG_VERSION")));
    r.line(format!("Adresse            : {}", cam.addr));
    r.line(format!("Nom PTP/IP         : {} (protocole {:#010x})", cam.peer_name, cam.peer_version));
    r.line(format!("Fabricant / modèle : {} / {}", info.manufacturer, info.model));
    r.line(format!("Version            : {}", info.device_version));
    r.line(format!("N° de série        : {}", mask_serial(&info.serial, args.has("--show-serial"))));
    r.line(format!(
        "Extension          : id {:#x}, v{}, « {} », mode {}",
        info.vendor_ext_id, info.vendor_ext_version, info.vendor_ext_desc, info.functional_mode
    ));
    r.line("");
    r.line("Fonctions (d'après les opérations annoncées) :");
    for (name, ops) in FEATURES {
        let ok = ops.iter().all(|o| info.operations.contains(o));
        r.line(format!("  [{}] {name}", if ok { "oui" } else { "non" }));
    }
    let zoom = info.properties.contains(&DPC_FOCAL_LENGTH);
    r.line(format!(
        "  [{}] Propriété FocalLength (piste pour le zoom ; voir « p900ctl props »)",
        if zoom { "oui" } else { "non" }
    ));
    r.line("");
    r.line(format!("Opérations ({}) :", info.operations.len()));
    let mut ops = info.operations.clone();
    ops.sort_unstable();
    for o in ops {
        r.line(format!("  {}", op_label(o)));
    }
    r.line(format!("Événements ({}) :", info.events.len()));
    for c in &info.events {
        r.line(format!("  {c:#06x}  {}", event_name(*c).unwrap_or("(inconnu)")));
    }
    r.line(format!("Propriétés ({}) :", info.properties.len()));
    for c in &info.properties {
        r.line(format!("  {c:#06x}  {}", prop_name(*c).unwrap_or("(Nikon ou inconnue)")));
    }
    if cam.supports(OC_NIKON_GET_VENDOR_PROP_CODES) {
        match cam.vendor_prop_codes() {
            Ok(v) => {
                r.line(format!("Propriétés Nikon cachées ({}) :", v.len()));
                for c in v {
                    r.line(format!("  {c:#06x}"));
                }
            }
            Err(x) => r.line(format!("Propriétés Nikon cachées : erreur ({x})")),
        }
    }
    let fmts = |v: &[u16]| v.iter().map(|c| format_name(*c).map(String::from).unwrap_or(format!("{c:#06x}"))).collect::<Vec<_>>().join(", ");
    r.line(format!("Formats de prise de vue : {}", fmts(&info.capture_formats)));
    r.line(format!("Formats d'image         : {}", fmts(&info.image_formats)));
    std::thread::sleep(Duration::from_millis(300));
    r.line(format!(
        "Canal d'événements : {} ; sondes de vie acquittées : {}",
        if cam.event_channel_alive() { "ouvert" } else { "fermé par l'appareil" },
        cam.probes_answered()
    ));
    Ok(())
}

// --- props -------------------------------------------------------------------------

fn form_text(f: &Form) -> String {
    match f {
        Form::None => String::new(),
        Form::Range { min, max, step } => format!("plage {min} … {max} (pas {step})"),
        Form::Enum(v) => {
            let shown: Vec<String> = v.iter().take(24).map(|x| x.to_string()).collect();
            let more = if v.len() > 24 { format!(" … (+{})", v.len() - 24) } else { String::new() };
            format!("valeurs {{{}}}{more}", shown.join(", "))
        }
        Form::Other(x) => format!("forme {x} non décodée"),
    }
}

fn desc_line(d: &PropDesc) -> String {
    format!(
        "{:#06x}  {:<26} {:<9} {:<16} actuel {}  {}",
        d.code,
        prop_name(d.code).unwrap_or("(Nikon ou inconnue)"),
        dataset::dtype_name(d.dtype),
        if d.writable { "lecture+écriture" } else { "lecture seule" },
        d.current,
        form_text(&d.form)
    )
}

pub(crate) fn all_props(cam: &mut Camera) -> Vec<u16> {
    let mut v = cam.info.as_ref().map(|i| i.properties.clone()).unwrap_or_default();
    if cam.supports(OC_NIKON_GET_VENDOR_PROP_CODES) {
        if let Ok(x) = cam.vendor_prop_codes() {
            for c in x {
                if !v.contains(&c) {
                    v.push(c);
                }
            }
        }
    }
    v
}

fn cmd_props(cam: &mut Camera, args: &Args) -> Res<()> {
    let mut r = Report::new(args)?;
    let codes = all_props(cam);
    r.line(format!("{} propriétés :", codes.len()));
    for c in codes {
        match cam.prop_desc(c) {
            Ok(d) => r.line(desc_line(&d)),
            Err(x) => r.line(format!("{c:#06x}  erreur : {x}")),
        }
    }
    Ok(())
}

// --- ls / get --------------------------------------------------------------------

fn select_handles(cam: &mut Camera, args: &Args, start: usize) -> Res<Vec<u32>> {
    if args.has("--all") || args.val("--last").is_some() || args.pos.len() <= start {
        let mut hs = cam.object_handles().map_err(e)?;
        if let Some(n) = args.val("--last") {
            let n: usize = n.parse().map_err(|_| "--last attend un nombre".to_string())?;
            let skip = hs.len().saturating_sub(n);
            hs = hs.split_off(skip);
        }
        return Ok(hs);
    }
    args.pos[start..]
        .iter()
        .map(|s| {
            let s = s.trim();
            match s.strip_prefix("0x") {
                Some(h) => u32::from_str_radix(h, 16),
                None => s.parse(),
            }
            .map_err(|_| format!("handle invalide : {s}"))
        })
        .collect()
}

fn cmd_ls(cam: &mut Camera, args: &Args) -> Res<()> {
    for id in cam.storage_ids().map_err(e)? {
        match cam.storage_info(id) {
            Ok(s) => println!(
                "Support {id:#010x} : {} — libre {:.1} Go sur {:.1} Go ({} vues)",
                if s.description.is_empty() { &s.volume_label } else { &s.description },
                s.free_bytes as f64 / 1e9,
                s.max_capacity as f64 / 1e9,
                s.free_images
            ),
            Err(x) => println!("Support {id:#010x} : {x}"),
        }
    }
    let hs = select_handles(cam, args, usize::MAX)?;
    println!("{} objets :", hs.len());
    for h in hs {
        match cam.object_info(h) {
            Ok(o) => println!(
                "  {h:#010x}  {:<14} {:>8} Ko  {}x{}  {}  {}",
                o.filename,
                o.size / 1024,
                o.width,
                o.height,
                o.capture_date,
                format_name(o.format).unwrap_or("")
            ),
            Err(x) => println!("  {h:#010x}  erreur : {x}"),
        }
    }
    Ok(())
}

struct FileSink {
    f: BufWriter<File>,
    total: u64,
    done: u64,
    last: Instant,
}

impl DataSink for FileSink {
    fn start(&mut self, total: u64) {
        self.total = total;
    }
    fn chunk(&mut self, b: &[u8]) -> io::Result<()> {
        self.f.write_all(b)?;
        self.done += b.len() as u64;
        if self.last.elapsed() > Duration::from_millis(500) {
            eprint!("\r    {} / {} Ko", self.done / 1024, self.total / 1024);
            self.last = Instant::now();
        }
        Ok(())
    }
}

/// Télécharge un objet ; écrit d'abord dans un fichier temporaire.
pub(crate) fn download(cam: &mut Camera, h: u32, dir: &Path, thumb: bool, force: bool) -> Res<Option<PathBuf>> {
    let info = cam.object_info(h).map_err(e)?;
    if info.format == OFC_ASSOCIATION {
        return Ok(None);
    }
    let name = if thumb { format!("thumb_{}", info.filename) } else { info.filename.clone() };
    let safe: String = name.chars().map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c }).collect();
    let path = dir.join(if safe.is_empty() { format!("objet_{h:08x}.jpg") } else { safe });
    if path.exists() && !force {
        eprintln!("  {} existe déjà, ignoré (--force pour écraser)", path.display());
        return Ok(Some(path));
    }
    let tmp = path.with_extension("part");
    let t0 = Instant::now();
    if thumb {
        let d = cam.get_thumb(h).map_err(e)?;
        fs::write(&tmp, d).map_err(e)?;
    } else {
        let mut sink = FileSink {
            f: BufWriter::new(File::create(&tmp).map_err(e)?),
            total: info.size as u64,
            done: 0,
            last: Instant::now(),
        };
        let res = cam.get_object(h, &mut sink);
        sink.f.flush().map_err(e)?;
        if let Err(x) = res {
            let _ = fs::remove_file(&tmp);
            return Err(e(x));
        }
        eprint!("\r");
    }
    fs::rename(&tmp, &path).map_err(e)?;
    let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let secs = t0.elapsed().as_secs_f64().max(0.001);
    eprintln!(
        "  {} : {} Ko en {:.1} s ({:.0} Ko/s)",
        path.display(),
        size / 1024,
        secs,
        size as f64 / 1024.0 / secs
    );
    Ok(Some(path))
}

fn cmd_get(cam: &mut Camera, args: &Args) -> Res<()> {
    let hs = select_handles(cam, args, 1)?;
    let dir = out_dir(args)?;
    for h in hs {
        if let Err(x) = download(cam, h, &dir, args.has("--thumb"), args.has("--force")) {
            eprintln!("  {h:#010x} : {x}");
        }
    }
    Ok(())
}

// --- capture -------------------------------------------------------------------------

pub(crate) fn capture_and_find(cam: &mut Camera, af: bool) -> Res<(Option<u32>, Duration)> {
    let before = cam.object_handles().ok();
    let t0 = Instant::now();
    let op = cam.capture(af).map_err(e)?;
    eprintln!("Déclenchement par {} envoyé", op_label(op));
    if let Err(x) = cam.wait_ready(Duration::from_secs(15)) {
        eprintln!("  attente « prêt » : {x}");
    }
    let mut h = cam.wait_object_added(Duration::from_secs(10)).map_err(e)?;
    if h.is_none() {
        if let (Some(b), Ok(after)) = (before, cam.object_handles()) {
            h = after.into_iter().rev().find(|x| !b.contains(x));
            if h.is_some() {
                eprintln!("  (pas d'événement ObjectAdded ; nouvelle photo trouvée par comparaison)");
            }
        }
    }
    Ok((h, t0.elapsed()))
}

fn cmd_capture(cam: &mut Camera, args: &Args) -> Res<()> {
    let (h, dt) = capture_and_find(cam, args.has("--af"))?;
    match h {
        Some(h) => {
            println!("Nouvelle photo {h:#010x}, disponible {} ms après l'ordre", dt.as_millis());
            if !args.has("--no-download") {
                download(cam, h, &out_dir(args)?, false, args.has("--force"))?;
            }
            Ok(())
        }
        None => Err("déclenchement accepté, mais aucune nouvelle photo détectée".into()),
    }
}

// --- liveview ------------------------------------------------------------------------

fn cmd_liveview(cam: &mut Camera, args: &Args) -> Res<()> {
    let n: usize = args.num("--frames", 50)?;
    let dir = match args.val("--out") {
        Some(_) => Some(out_dir(args)?),
        None => None,
    };
    cam.start_liveview().map_err(e)?;
    let t0 = Instant::now();
    let mut bytes = 0usize;
    let mut got = 0usize;
    for i in 0..n {
        let f = match cam.liveview_frame() {
            Ok(f) => f,
            Err(x) if x.response_code() == Some(RC_NIKON_NOT_LIVEVIEW) => {
                cam.start_liveview().map_err(e)?;
                continue;
            }
            Err(x) => {
                let _ = cam.end_liveview();
                return Err(e(x));
            }
        };
        bytes += f.len();
        got += 1;
        if let Some(d) = &dir {
            fs::write(d.join(format!("lv_{i:04}.jpg")), &f).map_err(e)?;
        }
    }
    let secs = t0.elapsed().as_secs_f64().max(0.001);
    let _ = cam.end_liveview();
    println!(
        "{got} images de visée en {secs:.1} s : {:.1} images/s, {:.1} Ko par image",
        got as f64 / secs,
        bytes as f64 / 1024.0 / got.max(1) as f64
    );
    Ok(())
}

// --- zoom / set-prop -------------------------------------------------------------------

/// Valeur de zoom suivante (sens > 0 : téléobjectif), d'après la forme de la propriété.
pub(crate) fn zoom_target(d: &PropDesc, dir: i32) -> Option<Value> {
    let cur = d.current.as_i128()?;
    match &d.form {
        Form::Enum(v) => {
            let mut nums: Vec<i128> = v.iter().filter_map(|x| x.as_i128()).collect();
            nums.sort_unstable();
            let next = if dir > 0 { nums.into_iter().find(|&x| x > cur) } else { nums.into_iter().rev().find(|&x| x < cur) };
            dataset::int_like(&d.current, next?)
        }
        Form::Range { min, max, step } => {
            let (lo, hi, st) = (min.as_i128()?, max.as_i128()?, step.as_i128()?.max(1));
            let jump = (((hi - lo) / 20) / st).max(1) * st;
            let t = (cur + jump * dir as i128).clamp(lo, hi);
            if t == cur {
                return None;
            }
            dataset::int_like(&d.current, t)
        }
        _ => None,
    }
}

/// Plage de zoom (focale, en 1/100 mm) : (min, max, actuelle, valeurs permises
/// si la propriété est une énumération, sinon vide).
pub(crate) fn zoom_info(d: &PropDesc) -> Option<(i128, i128, i128, Vec<i128>)> {
    let cur = d.current.as_i128()?;
    match &d.form {
        Form::Enum(v) => {
            let mut n: Vec<i128> = v.iter().filter_map(|x| x.as_i128()).collect();
            n.sort_unstable();
            Some((*n.first()?, *n.last()?, cur, n))
        }
        Form::Range { min, max, .. } => Some((min.as_i128()?, max.as_i128()?, cur, Vec::new())),
        _ => None,
    }
}

/// Valeur permise la plus proche de la focale voulue (glissière de zoom).
pub(crate) fn zoom_to(d: &PropDesc, want: i128) -> Option<Value> {
    match &d.form {
        Form::Enum(v) => {
            let best = v.iter().filter_map(|x| x.as_i128()).min_by_key(|x| (x - want).abs())?;
            dataset::int_like(&d.current, best)
        }
        Form::Range { min, max, step } => {
            let (lo, hi, st) = (min.as_i128()?, max.as_i128()?, step.as_i128()?.max(1));
            let t = (lo + (want.clamp(lo, hi) - lo + st / 2) / st * st).clamp(lo, hi);
            dataset::int_like(&d.current, t)
        }
        _ => None,
    }
}

fn cmd_zoom(cam: &mut Camera, args: &Args) -> Res<()> {
    let arg = args.pos.get(1).ok_or("zoom attend in, out ou une valeur")?;
    if !cam.info.as_ref().is_some_and(|i| i.properties.contains(&DPC_FOCAL_LENGTH)) {
        return Err("l'appareil n'annonce pas la propriété FocalLength (0x5008) : zoom non disponible \
                    par ce moyen. Lancez « p900ctl watch » et utilisez le zoom de l'appareil pour \
                    repérer la propriété qui change."
            .into());
    }
    let d = cam.prop_desc(DPC_FOCAL_LENGTH).map_err(e)?;
    println!("Avant : {}", desc_line(&d));
    if !d.writable {
        return Err("FocalLength est en lecture seule sur cet appareil".into());
    }
    let target = match arg.as_str() {
        "in" | "+" => zoom_target(&d, 1).ok_or("déjà au maximum")?,
        "out" | "-" => zoom_target(&d, -1).ok_or("déjà au minimum")?,
        v => dataset::parse_value(d.dtype, v)?,
    };
    cam.set_prop(DPC_FOCAL_LENGTH, &target).map_err(e)?;
    std::thread::sleep(Duration::from_millis(300));
    println!("Après : {}", cam.prop_value(DPC_FOCAL_LENGTH, d.dtype).map_err(e)?);
    Ok(())
}

fn parse_code(s: &str) -> Res<u16> {
    let t = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    u16::from_str_radix(t, 16).map_err(|_| format!("code de propriété invalide : {s} (hexadécimal attendu)"))
}

fn cmd_set_prop(cam: &mut Camera, args: &Args) -> Res<()> {
    let code = parse_code(args.pos.get(1).ok_or("set-prop attend CODE VALEUR")?)?;
    let raw = args.pos.get(2).ok_or("set-prop attend CODE VALEUR")?;
    let d = cam.prop_desc(code).map_err(e)?;
    println!("Avant : {}", desc_line(&d));
    if !d.writable && !args.has("--force") {
        return Err("propriété annoncée en lecture seule (--force pour essayer quand même)".into());
    }
    let v = dataset::parse_value(d.dtype, raw)?;
    if let Form::Enum(allowed) = &d.form {
        if !allowed.contains(&v) && !args.has("--force") {
            return Err(format!("{v} ne fait pas partie des valeurs annoncées (--force pour essayer)"));
        }
    }
    cam.set_prop(code, &v).map_err(e)?;
    println!("Après : {}", cam.prop_value(code, d.dtype).map_err(e)?);
    Ok(())
}

// --- watch -----------------------------------------------------------------------------

fn cmd_watch(cam: &mut Camera, args: &Args) -> Res<()> {
    let secs: u64 = args.num("--seconds", 30)?;
    let mut watched: Vec<(u16, u16, Option<Value>)> = Vec::new();
    for c in all_props(cam) {
        if let Ok(d) = cam.prop_desc(c) {
            watched.push((c, d.dtype, Some(d.current)));
        }
    }
    println!("Surveillance pendant {secs} s ({} propriétés). Manipulez l'appareil…", watched.len());
    let t0 = Instant::now();
    let mut last_poll = Instant::now() - Duration::from_secs(1);
    while t0.elapsed() < Duration::from_secs(secs) {
        for ev in cam.poll_events().map_err(e)? {
            println!(
                "[{:>6.1} s] événement {:#06x} {} {:?}",
                t0.elapsed().as_secs_f64(),
                ev.code,
                event_name(ev.code).unwrap_or("(inconnu)"),
                ev.params
            );
        }
        if last_poll.elapsed() >= Duration::from_millis(700) {
            last_poll = Instant::now();
            for (code, dtype, prev) in watched.iter_mut() {
                if let Ok(v) = cam.prop_value(*code, *dtype) {
                    if prev.as_ref() != Some(&v) {
                        println!(
                            "[{:>6.1} s] {:#06x} {} : {} → {}",
                            t0.elapsed().as_secs_f64(),
                            code,
                            prop_name(*code).unwrap_or(""),
                            prev.as_ref().map(|p| p.to_string()).unwrap_or_default(),
                            v
                        );
                        *prev = Some(v);
                    }
                }
            }
        }
        if !cam.event_channel_alive() {
            return Err("l'appareil a fermé le canal d'événements".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn desc(form: Form, cur: u32) -> PropDesc {
        PropDesc { code: DPC_FOCAL_LENGTH, dtype: dataset::DT_UINT32, writable: true, factory: Value::U32(430), current: Value::U32(cur), form }
    }

    #[test]
    fn zoom_steps_through_range_and_enum() {
        let r = Form::Range { min: Value::U32(430), max: Value::U32(35700), step: Value::U32(1) };
        assert_eq!(zoom_target(&desc(r.clone(), 430), 1), Some(Value::U32(430 + 1763)));
        assert_eq!(zoom_target(&desc(r.clone(), 430), -1), None);
        assert_eq!(zoom_target(&desc(r, 35000), 1), Some(Value::U32(35700)));
        let en = Form::Enum(vec![Value::U32(430), Value::U32(1000), Value::U32(5000)]);
        assert_eq!(zoom_target(&desc(en.clone(), 1000), 1), Some(Value::U32(5000)));
        assert_eq!(zoom_target(&desc(en, 1000), -1), Some(Value::U32(430)));
    }

    #[test]
    fn guid_and_serial_helpers() {
        assert_eq!(parse_guid("00112233-44556677-8899aabb-ccddeeff").unwrap()[15], 0xff);
        assert!(parse_guid("abc").is_err());
        assert_eq!(mask_serial("12345678", false), "****5678");
        assert_eq!(parse_code("0x5008").unwrap(), 0x5008);
    }
}
