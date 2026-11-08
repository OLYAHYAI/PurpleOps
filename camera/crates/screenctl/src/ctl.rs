//! Prise de commande locale pour le serveur MCP PurpleOps (pilotage par l'IA).
//!
//! Une prise Unix (`/data/local/kali/opt/purpleops-mcp/screenctl.sock`, droits 0600
//! root) déposée dans le dossier du serveur MCP, dans le chroot Kali : il y accède
//! sous `/opt/purpleops-mcp/screenctl.sock`. (Pas `/run` : souvent remplacé par un
//! montage au démarrage du chroot, la prise y deviendrait invisible d'un côté.) Les autres applis du téléphone n'y ont pas accès
//! (pas de port réseau, fichier root dans /data/local). Active seulement si
//! l'utilisateur a coché « Autoriser le pilotage par l'IA » dans l'onglet Écran.
//!
//! Protocole ligne à ligne. Requête : une commande. Réponse :
//! `OK <taille>\n<contenu>` ou `ERR <raison>\n`.
//!   size                       -> "L H" (écran en pixels)
//!   screenshot                 -> PNG de l'écran
//!   ui                         -> XML de l'arbre d'interface (uiautomator)
//!   tap X Y                    -> toucher
//!   swipe X1 Y1 X2 Y2 MS       -> glisser
//!   key CODE                   -> touche Android (4 Retour, 3 Accueil, 187 Applis…)
//!   text <base64 UTF-8>        -> taper du texte (aide Java : accents compris)
//! Chaque commande est inscrite au journal SOC (« ia … »), le texte tapé par sa
//! seule longueur.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{b64_decode, display_size, input, Helper, Logger};

pub fn spawn(path: PathBuf, log: Arc<Mutex<Logger>>) {
    thread::spawn(move || {
        let logl = |m: &str| log.lock().unwrap_or_else(|p| p.into_inner()).line(m);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(l) => l,
            Err(e) => {
                logl(&format!("ia : prise {} impossible : {e}", path.display()));
                return;
            }
        };
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        logl(&format!("ia : pilotage autorisé, prise {}", path.display()));
        let helper: Arc<Mutex<Option<Helper>>> = Arc::new(Mutex::new(None));
        for conn in listener.incoming().flatten() {
            let (log, helper) = (log.clone(), helper.clone());
            thread::spawn(move || {
                let _ = handle(conn, &log, &helper);
            });
        }
    });
}

fn handle(s: UnixStream, log: &Arc<Mutex<Logger>>, helper: &Arc<Mutex<Option<Helper>>>) -> std::io::Result<()> {
    let logl = |m: &str| log.lock().unwrap_or_else(|p| p.into_inner()).line(m);
    let mut w = s.try_clone()?;
    let rd = BufReader::new(s);
    for line in rd.lines() {
        let line = line?;
        let mut it = line.split_whitespace();
        let cmd = it.next().unwrap_or("");
        let args: Vec<&str> = it.collect();
        let res: Result<Vec<u8>, String> = run(cmd, &args, log, helper);
        match &res {
            Ok(_) => logl(&format!("ia {}", describe(cmd, &args))),
            Err(e) => logl(&format!("ia {} refusé : {e}", describe(cmd, &args))),
        }
        match res {
            Ok(data) => {
                w.write_all(format!("OK {}\n", data.len()).as_bytes())?;
                w.write_all(&data)?;
            }
            Err(e) => w.write_all(format!("ERR {}\n", e.replace('\n', " ")).as_bytes())?,
        }
        w.flush()?;
    }
    Ok(())
}

/// Description pour le journal : jamais le texte tapé lui-même.
fn describe(cmd: &str, args: &[&str]) -> String {
    match cmd {
        "text" => format!("text ({} car.)", args.first().and_then(|a| b64_decode(a)).map(|b| String::from_utf8_lossy(&b).chars().count()).unwrap_or(0)),
        _ => format!("{cmd} {}", args.join(" ")).trim().to_string(),
    }
}

fn nums<const N: usize>(args: &[&str]) -> Result<[u32; N], String> {
    if args.len() != N {
        return Err(format!("{N} nombres attendus"));
    }
    let mut out = [0u32; N];
    for (o, a) in out.iter_mut().zip(args) {
        *o = a.parse().map_err(|_| format!("nombre invalide : {a}"))?;
    }
    Ok(out)
}

fn run(cmd: &str, args: &[&str], log: &Arc<Mutex<Logger>>, helper: &Arc<Mutex<Option<Helper>>>) -> Result<Vec<u8>, String> {
    let (w, h) = display_size();
    let inside = |x: u32, y: u32| -> Result<(), String> {
        if x < w && y < h {
            Ok(())
        } else {
            Err(format!("({x},{y}) hors de l'écran {w}x{h}"))
        }
    };
    match cmd {
        "size" => Ok(format!("{w} {h}").into_bytes()),
        "screenshot" => {
            let out = Command::new("screencap").arg("-p").stdin(Stdio::null()).stderr(Stdio::null()).output()
                .map_err(|e| format!("screencap : {e}"))?;
            if out.stdout.len() < 8 {
                return Err("capture vide (écran sécurisé ?)".into());
            }
            Ok(out.stdout)
        }
        "ui" => {
            let f = "/data/local/tmp/screenctl-ui.xml";
            let _ = std::fs::remove_file(f);
            let _ = Command::new("uiautomator").args(["dump", f]).stdin(Stdio::null())
                .stdout(Stdio::null()).stderr(Stdio::null()).status();
            let xml = std::fs::read(f).map_err(|_| "uiautomator n'a rien produit (écran en transition ? réessayer)".to_string())?;
            let _ = std::fs::remove_file(f);
            Ok(xml)
        }
        "tap" => {
            let [x, y] = nums::<2>(args)?;
            inside(x, y)?;
            input(&["tap", &x.to_string(), &y.to_string()]);
            Ok(Vec::new())
        }
        "swipe" => {
            let [x1, y1, x2, y2, ms] = nums::<5>(args)?;
            inside(x1, y1)?;
            inside(x2, y2)?;
            let ms = ms.clamp(50, 5000);
            let a: Vec<String> = [x1, y1, x2, y2, ms].iter().map(u32::to_string).collect();
            input(&["swipe", &a[0], &a[1], &a[2], &a[3], &a[4]]);
            Ok(Vec::new())
        }
        "key" => {
            let [code] = nums::<1>(args)?;
            if code == 0 || code > 400 {
                return Err(format!("code de touche invalide : {code}"));
            }
            input(&["keyevent", &code.to_string()]);
            Ok(Vec::new())
        }
        "text" => {
            let raw = args.first().and_then(|a| b64_decode(a)).ok_or("texte base64 attendu")?;
            if raw.len() > 4096 {
                return Err("texte trop long (4096 octets max)".into());
            }
            // Aide Java (démarrée à la première frappe, ~1 s) : texte Unicode exact.
            {
                let mut g = helper.lock().unwrap_or_else(|p| p.into_inner());
                if g.is_none() {
                    *g = Some(Helper::start(None, log.clone()));
                }
            }
            for _ in 0..30 {
                if helper.lock().unwrap_or_else(|p| p.into_inner()).as_ref().is_some_and(|hp| hp.send('T', &raw)) {
                    return Ok(Vec::new());
                }
                thread::sleep(Duration::from_millis(200));
            }
            // Repli : commande `input text` (ASCII seulement, espaces en %s).
            let t = String::from_utf8_lossy(&raw);
            if !t.is_ascii() {
                return Err("aide Java indisponible : seul le texte ASCII peut être tapé".into());
            }
            input(&["text", &t.replace(' ', "%s")]);
            Ok(Vec::new())
        }
        "" => Err("commande vide".into()),
        other => Err(format!("commande inconnue : {other}")),
    }
}
