//! Interface locale : visée MJPEG et boutons dans le navigateur du téléphone.
//!
//! Écoute par défaut sur 127.0.0.1 : le navigateur Android et le chroot partagent
//! le même réseau, rien n'est exposé ailleurs. Chaque requête porte un jeton
//! aléatoire, pour qu'une autre page ouverte dans le navigateur ne puisse pas
//! déclencher l'appareil.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use ptpip::codes::*;
use ptpip::dataset::{self, Form, PropDesc, Value};
use ptpip::Camera;

use crate::{all_props, capture_and_find, download, hex, random_bytes, zoom_info, zoom_target, zoom_to};

#[derive(Default)]
struct Frame {
    seq: u64,
    jpeg: Arc<Vec<u8>>,
    fps: f64,
}

#[derive(Default)]
struct Status {
    last_error: String,
    last_file: String,
    thumb: Arc<Vec<u8>>,
}

struct Shared {
    cam: Mutex<Camera>,
    frame: Mutex<Frame>,
    frame_cv: Condvar,
    viewers: AtomicUsize,
    status: Mutex<Status>,
    token: String,
    out_dir: PathBuf,
    model: String,
    zoom_ok: bool,
    liveview_ok: bool,
    focus_ok: bool,
    thumbs: Mutex<HashMap<u32, Arc<Vec<u8>>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn serve(cam: Camera, listen: SocketAddr, out_dir: PathBuf) -> Result<(), String> {
    let model = cam.info.as_ref().map(|i| i.model.clone()).unwrap_or_default();
    let liveview_ok = [OC_NIKON_START_LIVEVIEW, OC_NIKON_GET_LIVEVIEW_IMG].iter().all(|o| cam.supports(*o));
    let mut cam = cam;
    let zoom_ok = cam.info.as_ref().is_some_and(|i| i.properties.contains(&DPC_FOCAL_LENGTH))
        && cam.prop_desc(DPC_FOCAL_LENGTH).map(|d| d.writable).unwrap_or(false);
    let focus_ok = cam.supports(OC_NIKON_AF_DRIVE);
    let token = hex(&random_bytes(16).map_err(|x| format!("jeton : {x}"))?);
    let sh = Arc::new(Shared {
        cam: Mutex::new(cam),
        frame: Mutex::new(Frame::default()),
        frame_cv: Condvar::new(),
        viewers: AtomicUsize::new(0),
        status: Mutex::new(Status::default()),
        token,
        out_dir,
        model,
        zoom_ok,
        liveview_ok,
        focus_ok,
        thumbs: Mutex::new(HashMap::new()),
    });
    let listener = TcpListener::bind(listen).map_err(|x| format!("écoute sur {listen} : {x}"))?;
    if !listen.ip().is_loopback() {
        eprintln!("attention : écoute hors de 127.0.0.1, l'interface est joignable depuis le réseau ({listen})");
    }
    println!("Ouvrez dans le navigateur du téléphone : http://{listen}/?t={}", sh.token);
    println!("Visée {} ; zoom {}. Ctrl+C pour arrêter.",
        if liveview_ok { "disponible" } else { "non annoncée par l'appareil" },
        if zoom_ok { "disponible" } else { "indisponible (FocalLength absente ou en lecture seule)" });
    if liveview_ok {
        let s = sh.clone();
        thread::spawn(move || liveview_loop(s));
    }
    for conn in listener.incoming().flatten() {
        let s = sh.clone();
        thread::spawn(move || {
            let _ = handle(conn, s);
        });
    }
    Ok(())
}

fn liveview_loop(sh: Arc<Shared>) {
    let mut running = false;
    let mut last = Instant::now();
    loop {
        if sh.viewers.load(Ordering::SeqCst) == 0 {
            if running {
                let _ = lock(&sh.cam).end_liveview();
                running = false;
            }
            thread::sleep(Duration::from_millis(200));
            continue;
        }
        let res = {
            let mut cam = lock(&sh.cam);
            if !running {
                match cam.start_liveview() {
                    Ok(()) => running = true,
                    Err(x) => lock(&sh.status).last_error = format!("visée : {x}"),
                }
            }
            if running { Some(cam.liveview_frame()) } else { None }
        };
        match res {
            Some(Ok(jpeg)) => {
                let dt = last.elapsed().as_secs_f64().max(0.001);
                last = Instant::now();
                let mut f = lock(&sh.frame);
                f.seq += 1;
                f.jpeg = Arc::new(jpeg);
                f.fps = if f.fps == 0.0 { 1.0 / dt } else { 0.8 * f.fps + 0.2 / dt };
                drop(f);
                sh.frame_cv.notify_all();
            }
            Some(Err(x)) => {
                running = false;
                if x.response_code() != Some(RC_NIKON_NOT_LIVEVIEW) {
                    lock(&sh.status).last_error = format!("visée : {x}");
                    thread::sleep(Duration::from_millis(800));
                }
            }
            None => thread::sleep(Duration::from_millis(1000)),
        }
        // Au plus ~25 images/s : laisse passer les commandes entre deux images.
        thread::sleep(Duration::from_millis(15));
    }
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn respond(s: &mut TcpStream, code: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        s,
        "HTTP/1.1 {code}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    s.write_all(body)
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn query_param<'a>(q: &'a str, key: &str) -> Option<&'a str> {
    q.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

struct ViewerGuard<'a>(&'a AtomicUsize);
impl Drop for ViewerGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

fn handle(mut s: TcpStream, sh: Arc<Shared>) -> std::io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut rd = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    rd.read_line(&mut line)?;
    loop {
        let mut h = String::new();
        if rd.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !query_param(query, "t").is_some_and(|t| same(t, &sh.token)) {
        return respond(&mut s, "403 Forbidden", "text/plain; charset=utf-8", "jeton absent ou invalide\n".as_bytes());
    }
    match (method, path) {
        ("GET", "/") => {
            let page = PAGE.replace("__TOKEN__", &sh.token).replace("__MODEL__", &html_escape(&sh.model));
            respond(&mut s, "200 OK", "text/html; charset=utf-8", page.as_bytes())
        }
        ("GET", "/stream.mjpg") => stream(s, &sh),
        ("GET", "/last.jpg") => {
            let t = lock(&sh.status).thumb.clone();
            if t.is_empty() {
                respond(&mut s, "404 Not Found", "text/plain", b"")
            } else {
                respond(&mut s, "200 OK", "image/jpeg", &t)
            }
        }
        ("GET", "/api/status") => {
            let f = lock(&sh.frame);
            let st = lock(&sh.status);
            let body = format!(
                "{{\"model\":{},\"liveview\":{},\"zoom\":{},\"focus\":{},\"fps\":{:.1},\"frames\":{},\"last_file\":{},\"last_error\":{}}}",
                json_str(&sh.model), sh.liveview_ok, sh.zoom_ok, sh.focus_ok, f.fps, f.seq,
                json_str(&st.last_file), json_str(&st.last_error)
            );
            drop((f, st));
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("POST", "/api/capture") => {
            let r = {
                let mut cam = lock(&sh.cam);
                capture_af(&mut cam).and_then(|(h, dt)| {
                    let h = h.ok_or("aucune nouvelle photo détectée")?;
                    let thumb = cam.get_thumb(h).unwrap_or_default();
                    let file = download(&mut cam, h, &sh.out_dir, false, false)?
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    Ok((file, thumb, dt))
                })
            };
            let body = match r {
                Ok((file, thumb, dt)) => {
                    let mut st = lock(&sh.status);
                    st.last_file = file.clone();
                    st.thumb = Arc::new(thumb);
                    format!("{{\"ok\":true,\"file\":{},\"ms\":{}}}", json_str(&file), dt.as_millis())
                }
                Err(m) => {
                    lock(&sh.status).last_error = format!("déclenchement : {m}");
                    format!("{{\"ok\":false,\"error\":{}}}", json_str(&m))
                }
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("GET", "/api/zoominfo") => {
            let r = if !sh.zoom_ok {
                None
            } else {
                lock(&sh.cam).prop_desc(DPC_FOCAL_LENGTH).ok().and_then(|d| zoom_info(&d))
            };
            let body = match r {
                Some((min, max, cur, vals)) => format!(
                    "{{\"ok\":true,\"min\":{min},\"max\":{max},\"cur\":{cur},\"values\":[{}]}}",
                    vals.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",")
                ),
                None => "{\"ok\":false}".to_string(),
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("POST", "/api/zoom") => {
            let dir = if query_param(query, "dir") == Some("in") { 1 } else { -1 };
            // ?value=N : focale visée (glissière) ; sinon un cran (?dir=in|out).
            let want: Option<i128> = query_param(query, "value").and_then(|v| v.parse().ok());
            let r: Result<String, String> = if !sh.zoom_ok {
                Err("zoom indisponible sur cet appareil".into())
            } else {
                let mut cam = lock(&sh.cam);
                cam.prop_desc(DPC_FOCAL_LENGTH)
                    .map_err(|x| x.to_string())
                    .and_then(|d| {
                        let v = match want {
                            Some(w) => zoom_to(&d, w).ok_or("focale hors plage")?,
                            None => zoom_target(&d, dir).ok_or("limite atteinte")?,
                        };
                        cam.set_prop(DPC_FOCAL_LENGTH, &v).map_err(|x| x.to_string())?;
                        Ok(v.to_string())
                    })
            };
            let body = match r {
                Ok(v) => format!("{{\"ok\":true,\"focal\":{}}}", json_str(&v)),
                Err(m) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&m)),
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("GET", "/api/photos") => {
            let off = qnum(query, "offset").unwrap_or(0) as usize;
            let cnt = qnum(query, "count").unwrap_or(18).min(48) as usize;
            let r: Result<String, String> = (|| {
                let mut cam = lock(&sh.cam);
                let mut hs = cam.object_handles().map_err(|x| x.to_string())?;
                hs.reverse();
                let mut items = Vec::new();
                for h in hs.iter().skip(off).take(cnt) {
                    if let Ok(o) = cam.object_info(*h) {
                        if o.format == OFC_ASSOCIATION {
                            continue;
                        }
                        items.push(format!(
                            "{{\"h\":{h},\"name\":{},\"size\":{},\"w\":{},\"ht\":{},\"date\":{},\"movie\":{}}}",
                            json_str(&o.filename), o.size, o.width, o.height, json_str(&o.capture_date),
                            o.format == 0x300D || o.format == 0x300A
                        ));
                    }
                }
                Ok(format!("{{\"total\":{},\"items\":[{}]}}", hs.len(), items.join(",")))
            })();
            match r {
                Ok(b) => respond(&mut s, "200 OK", "application/json", b.as_bytes()),
                Err(m) => respond(&mut s, "200 OK", "application/json", format!("{{\"error\":{}}}", json_str(&m)).as_bytes()),
            }
        }
        ("GET", "/api/thumb") => {
            let h = qnum(query, "h").unwrap_or(0);
            let cached = lock(&sh.thumbs).get(&h).cloned();
            let t = match cached {
                Some(t) => Some(t),
                None => lock(&sh.cam).get_thumb(h).ok().map(|d| {
                    let d = Arc::new(d);
                    lock(&sh.thumbs).insert(h, d.clone());
                    d
                }),
            };
            match t {
                Some(t) if !t.is_empty() => respond(&mut s, "200 OK", "image/jpeg", &t),
                _ => respond(&mut s, "404 Not Found", "text/plain", b""),
            }
        }
        ("GET", "/api/photo") => {
            let h = qnum(query, "h").unwrap_or(0);
            let mut data = Vec::new();
            let r = lock(&sh.cam).get_object(h, &mut data);
            match r {
                Ok(()) => respond(&mut s, "200 OK", "image/jpeg", &data),
                Err(x) => respond(&mut s, "502 Bad Gateway", "text/plain; charset=utf-8", x.to_string().as_bytes()),
            }
        }
        ("POST", "/api/save") => {
            let h = qnum(query, "h").unwrap_or(0);
            let r = {
                let mut cam = lock(&sh.cam);
                download(&mut cam, h, &sh.out_dir, false, false)
            };
            let body = match r {
                Ok(p) => format!("{{\"ok\":true,\"file\":{}}}", json_str(&p.map(|p| p.display().to_string()).unwrap_or_default())),
                Err(m) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&m)),
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("POST", "/api/delete") => {
            let h = qnum(query, "h").unwrap_or(0);
            let r = lock(&sh.cam).delete_object(h);
            lock(&sh.thumbs).remove(&h);
            let body = match r {
                Ok(()) => "{\"ok\":true}".to_string(),
                Err(x) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&x.to_string())),
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("POST", "/api/focus") => {
            let r: Result<(), String> = if !sh.focus_ok {
                Err("mise au point à distance non annoncée par l'appareil".into())
            } else {
                lock(&sh.cam).af_drive().map_err(|x| x.to_string())
            };
            let body = match r {
                Ok(()) => "{\"ok\":true,\"msg\":\"Mise au point faite\"}".to_string(),
                Err(m) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&m)),
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("GET", "/api/props") => {
            let items: Vec<String> = {
                let mut cam = lock(&sh.cam);
                let codes = all_props(&mut cam);
                codes.into_iter().filter(|c| !HIDDEN_PROPS.contains(c)).filter_map(|c| cam.prop_desc(c).ok()).map(|d| prop_json(&d)).collect()
            };
            respond(&mut s, "200 OK", "application/json", format!("[{}]", items.join(",")).as_bytes())
        }
        ("POST", "/api/prop") => {
            let r = set_prop_from_query(&sh, query);
            let body = match r {
                Ok(v) => format!("{{\"ok\":true,\"current\":{}}}", value_json(&v)),
                Err(m) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&m)),
            };
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        _ => respond(&mut s, "404 Not Found", "text/plain", b""),
    }
}

/// Déclenche avec autofocus ; si l'appareil refuse ce paramètre ou ne fait
/// pas le point, nouvel essai sans autofocus.
fn capture_af(cam: &mut Camera) -> Result<(Option<u32>, Duration), String> {
    match capture_and_find(cam, true) {
        Err(m) if m.contains("InitiateCapture") => {
            eprintln!("déclenchement avec autofocus refusé ({m}) ; nouvel essai sans autofocus");
            capture_and_find(cam, false)
        }
        r => r,
    }
}

fn qnum(query: &str, key: &str) -> Option<u32> {
    let v = query_param(query, key)?;
    match v.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => v.parse().ok(),
    }
}

/// Réglages jamais proposés : identifiant d'appairage Wi-Fi Nikon (une mauvaise
/// valeur pourrait bloquer la connexion). L'horloge a son propre bouton.
const HIDDEN_PROPS: &[u16] = &[0xD24F];

fn value_json(v: &Value) -> String {
    match v {
        Value::Str(s) => json_str(s),
        v => match v.as_i128() {
            Some(n) => n.to_string(),
            None => json_str(&v.to_string()),
        },
    }
}

fn prop_json(d: &PropDesc) -> String {
    let form = match &d.form {
        Form::Range { min, max, step } => format!(
            "{{\"kind\":\"range\",\"min\":{},\"max\":{},\"step\":{}}}",
            value_json(min), value_json(max), value_json(step)
        ),
        Form::Enum(v) => format!(
            "{{\"kind\":\"enum\",\"values\":[{}]}}",
            v.iter().map(value_json).collect::<Vec<_>>().join(",")
        ),
        _ => "{\"kind\":\"none\"}".to_string(),
    };
    format!(
        "{{\"code\":{},\"hex\":\"{:#06x}\",\"name\":{},\"type\":{},\"writable\":{},\"current\":{},\"form\":{}}}",
        d.code,
        d.code,
        json_str(prop_name(d.code).unwrap_or("")),
        json_str(&dataset::dtype_name(d.dtype)),
        d.writable,
        value_json(&d.current),
        form
    )
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hexv = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
                match hexv(b[i + 1]).zip(hexv(b[i + 2])).map(|(h, l)| h * 16 + l).ok_or(()) {
                    Ok(x) => {
                        out.push(x);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn set_prop_from_query(sh: &Shared, query: &str) -> Result<Value, String> {
    let code: u16 = query_param(query, "code")
        .and_then(|c| c.parse().ok())
        .ok_or("paramètre code manquant")?;
    let raw = percent_decode(query_param(query, "value").ok_or("paramètre value manquant")?);
    let mut cam = lock(&sh.cam);
    if HIDDEN_PROPS.contains(&code) {
        return Err("réglage protégé (appairage Wi-Fi)".into());
    }
    let d = cam.prop_desc(code).map_err(|x| x.to_string())?;
    if !d.writable {
        return Err("réglage en lecture seule sur cet appareil".into());
    }
    let v = dataset::parse_value(d.dtype, &raw)?;
    cam.set_prop(code, &v).map_err(|x| x.to_string())?;
    thread::sleep(Duration::from_millis(200));
    cam.prop_value(code, d.dtype).map_err(|x| x.to_string())
}

fn stream(mut s: TcpStream, sh: &Shared) -> std::io::Result<()> {
    s.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\n\
          Cache-Control: no-store\r\nConnection: close\r\n\r\n",
    )?;
    sh.viewers.fetch_add(1, Ordering::SeqCst);
    let _g = ViewerGuard(&sh.viewers);
    let mut seen = 0u64;
    loop {
        let jpeg = {
            let mut f = lock(&sh.frame);
            while f.seq == seen {
                let (g, t) = sh.frame_cv.wait_timeout(f, Duration::from_secs(2)).unwrap_or_else(|p| p.into_inner());
                f = g;
                if t.timed_out() {
                    break;
                }
            }
            if f.seq == seen {
                continue;
            }
            seen = f.seq;
            f.jpeg.clone()
        };
        write!(s, "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n", jpeg.len())?;
        s.write_all(&jpeg)?;
        s.write_all(b"\r\n")?;
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

const PAGE: &str = r##"<!doctype html>
<html lang="fr"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>p900ctl</title>
<style>
/* Interface façon appareil photo de téléphone : visée en grand sur fond noir,
   zoom en glissière, gros déclencheur, miniature de la dernière photo. */
:root{--fg:#fff;--mute:#9b9b9b;--chip:#ffffff1f;--acc:#ff3b4e;--sheet:#161616}
*{box-sizing:border-box;-webkit-tap-highlight-color:transparent}
html,body{height:100%;margin:0;background:#000;color:var(--fg);font:15px/1.35 system-ui,sans-serif;overflow:hidden}
body{display:flex;flex-direction:column}
#top{display:flex;justify-content:space-between;align-items:center;padding:10px 14px;font-size:13px;color:var(--mute)}
#top b{color:var(--fg);font-weight:600}
#gear{background:none;border:0;color:var(--fg);font-size:20px;padding:4px 8px}
#view{position:relative;flex:1;min-height:0;display:flex;align-items:center;justify-content:center;background:#000}
#view img{max-width:100%;max-height:100%;object-fit:contain}
#off{position:absolute;color:var(--mute);font-size:14px}
#ring{position:absolute;width:76px;height:76px;margin:-38px 0 0 -38px;border:2px solid #fff;border-radius:50%;opacity:0;pointer-events:none;transition:opacity .2s,transform .3s}
#ring.on{opacity:1;transform:scale(.8)}#ring.ok{border-color:#5ce17a}#ring.ko{border-color:var(--acc)}
#flash{position:absolute;inset:0;background:#fff;opacity:0;pointer-events:none;transition:opacity .25s}
#toast{position:absolute;left:50%;bottom:12px;transform:translateX(-50%);background:#000b;padding:6px 12px;border-radius:14px;font-size:13px;opacity:0;transition:opacity .3s;max-width:90%;text-align:center}
#toast.on{opacity:1}
#zoom{padding:10px 18px 4px}
#zl{text-align:center;font-weight:600;font-variant-numeric:tabular-nums;margin-bottom:2px}
input[type=range]{width:100%;accent-color:#fff;height:28px}
#chips{display:flex;justify-content:center;gap:8px;margin-top:6px}
#chips button{background:var(--chip);color:var(--fg);border:0;border-radius:16px;padding:6px 12px;font:inherit;font-size:13px;font-variant-numeric:tabular-nums}
#chips button.sel{background:#fff;color:#000;font-weight:600}
#ctrl{display:grid;grid-template-columns:1fr auto 1fr;align-items:center;padding:14px 26px 22px}
#thumb{width:56px;height:56px;border-radius:12px;background:#222 center/cover no-repeat;border:2px solid #333}
#shoot{width:78px;height:78px;border-radius:50%;border:4px solid #fff;background:#fff;box-shadow:inset 0 0 0 4px #000;justify-self:center}
#shoot:active{transform:scale(.94)}#shoot:disabled{background:#777}
#af{justify-self:end;width:56px;height:56px;border-radius:50%;border:0;background:var(--chip);color:var(--fg);font:inherit;font-size:13px;font-weight:600}
button:disabled{opacity:.35}
.sheet{position:fixed;inset:auto 0 0 0;max-height:78%;overflow:auto;background:var(--sheet);border-radius:18px 18px 0 0;padding:14px 16px 24px;transform:translateY(105%);transition:transform .25s}
.sheet.on{transform:none}
.sheet h2{font-size:16px;margin:0 0 8px;display:flex;justify-content:space-between}
.sheet h2 button{background:none;border:0;color:var(--fg);font-size:18px}
.row{border-top:1px solid #2a2a2a;padding:10px 0}.row b{font-weight:600}.row small{color:var(--mute);margin-left:6px}
.ctl{display:flex;gap:8px;margin-top:6px}
.ctl select,.ctl input{flex:1;font:inherit;padding:8px;border-radius:8px;border:1px solid #333;background:#222;color:var(--fg)}
.ctl button{font:inherit;font-size:14px;padding:8px 14px;border-radius:8px;border:0;background:#fff;color:#000}
#big img{width:100%;border-radius:10px}#big p{color:var(--mute);font-size:13px}
</style></head><body>
<div id="top"><b>__MODEL__</b><span id="st">…</span><button id="gear" aria-label="Réglages">⚙</button></div>
<div id="view"><img id="lv" alt=""><span id="off">Connexion à la visée…</span><div id="ring"></div><div id="flash"></div><div id="toast"></div></div>
<div id="zoom" hidden><div id="zl">1,0×</div><input id="zs" type="range" min="0" max="1000" value="0"><div id="chips"></div></div>
<div id="ctrl"><button id="thumb" aria-label="Dernière photo"></button><button id="shoot" aria-label="Déclencher"></button><button id="af">AF</button></div>
<div class="sheet" id="props"><h2>Réglages de l'appareil<button data-close>✕</button></h2><div id="pl">…</div></div>
<div class="sheet" id="big"><h2>Dernière photo<button data-close>✕</button></h2><img id="bigimg" alt=""><p id="bigtxt"></p></div>
<script>
const T="__TOKEN__",q=p=>p+(p.includes("?")?"&":"?")+"t="+T,$=id=>document.getElementById(id);
let Z=null,lastFile="",tt=0;
function toast(m){const t=$("toast");t.textContent=m;t.classList.add("on");clearTimeout(tt);tt=setTimeout(()=>t.classList.remove("on"),2500)}
const fr=x=>x.toFixed(1).replace(".",",");
async function api(p,post){const r=await fetch(q(p),post?{method:"POST"}:{});return r.json()}
async function st(){try{const s=await api("/api/status");
$("st").textContent=s.liveview?fr(s.fps)+" i/s":"visée indisponible";$("af").disabled=!s.focus;
if(s.last_error)toast(s.last_error);
if(s.liveview&&!$("lv").src){$("lv").src=q("/stream.mjpg")}
if(!Z&&s.zoom)zinfo()}catch(e){$("st").textContent="hors ligne"}}
$("lv").onload=()=>$("off").hidden=true;
/* Zoom : glissière logarithmique (1× … 83× sur la P900), raccourcis, focale réelle. */
const toS=f=>Math.round(1000*Math.log(f/Z.min)/Math.log(Z.max/Z.min)),toF=s=>Z.min*Math.pow(Z.max/Z.min,s/1000);
function showZ(f){$("zl").textContent=fr(f/Z.min)+"×";document.querySelectorAll("#chips button").forEach(b=>b.classList.toggle("sel",Math.abs(+b.dataset.x-f/Z.min)<.15*+b.dataset.x))}
async function zinfo(){const r=await api("/api/zoominfo");if(!r.ok)return;Z=r;$("zoom").hidden=false;
const top=Z.max/Z.min,c=$("chips");c.textContent="";[1,2,5,10,25,top].filter((x,i,a)=>x<=top&&a.indexOf(x)==i).forEach(x=>{const b=document.createElement("button");
b.dataset.x=x;b.textContent=(x==top?Math.round(x):x)+"×";b.onclick=()=>zoomTo(Z.min*x);c.appendChild(b)});$("zs").value=toS(Z.cur);showZ(Z.cur)}
$("zs").oninput=e=>Z&&showZ(toF(+e.target.value));
$("zs").onchange=e=>Z&&zoomTo(toF(+e.target.value));
async function zoomTo(f){$("zs").value=toS(f);showZ(f);const r=await api("/api/zoom?value="+Math.round(f),true);
if(r.ok){Z.cur=parseFloat(r.focal);$("zs").value=toS(Z.cur);showZ(Z.cur)}else toast(r.error)}
/* Déclencheur : éclair blanc, vibration, miniature mise à jour. */
$("shoot").onclick=async()=>{const b=$("shoot");b.disabled=true;$("flash").style.opacity=.8;setTimeout(()=>$("flash").style.opacity=0,120);
if(navigator.vibrate)navigator.vibrate(30);const r=await api("/api/capture",true);
if(r.ok){lastFile=r.file;const u=q("/last.jpg?"+Date.now());$("thumb").style.backgroundImage="url('"+u+"')";$("bigimg").src=u;toast("Enregistrée ("+r.ms+" ms)")}else toast(r.error);
b.disabled=false;$("lv").src="";setTimeout(()=>{$("lv").src=q("/stream.mjpg")},300)};
/* Toucher la visée = mise au point (anneau vert si réussie). */
async function focus(x,y){const g=$("ring");g.className="";if(x!=null){g.style.left=x+"px";g.style.top=y+"px"}else{g.style.left="50%";g.style.top="50%"}
g.classList.add("on");const r=await api("/api/focus",true);g.classList.add(r.ok?"ok":"ko");if(!r.ok)toast(r.error);setTimeout(()=>g.className="",700)}
$("view").onclick=e=>{if($("af").disabled)return;const b=$("view").getBoundingClientRect();focus(e.clientX-b.left,e.clientY-b.top)};
$("af").onclick=()=>focus();
/* Panneaux du bas : dernière photo, réglages. */
document.querySelectorAll("[data-close]").forEach(b=>b.onclick=()=>b.closest(".sheet").classList.remove("on"));
$("thumb").onclick=()=>{if(!lastFile)return toast("Pas encore de photo dans cette session");$("bigtxt").textContent=lastFile;$("big").classList.add("on")};
const U={20487:v=>"f/"+(v/100),20488:v=>(v/100)+" mm",20493:v=>v>=10000?(v/10000)+" s":"1/"+Math.round(10000/v)+" s",20496:v=>(v/1000)+" IL"};
const fmt=(c,v)=>U[c]&&typeof v=="number"?v+" ("+U[c](v)+")":String(v);
function row(p){const d=document.createElement("div");d.className="row";
const t=document.createElement("div");const b=document.createElement("b");b.textContent=p.name||p.hex;t.appendChild(b);
const sm=document.createElement("small");sm.textContent=p.hex+" · "+fmt(p.code,p.current)+(p.writable?"":" · lecture seule");t.appendChild(sm);d.appendChild(t);
if(p.writable&&p.form.kind!="none"){const c=document.createElement("div");c.className="ctl";let inp;
if(p.form.kind=="enum"){inp=document.createElement("select");p.form.values.forEach(v=>{const o=document.createElement("option");o.value=v;o.textContent=fmt(p.code,v);if(v===p.current)o.selected=true;inp.appendChild(o)})}
else{inp=document.createElement("input");inp.type="number";inp.min=p.form.min;inp.max=p.form.max;inp.step=p.form.step;inp.value=p.current}
const go=document.createElement("button");go.textContent="OK";
go.onclick=async()=>{const r=await api("/api/prop?code="+p.code+"&value="+encodeURIComponent(inp.value),true);
if(r.ok){p.current=r.current;sm.textContent=p.hex+" · "+fmt(p.code,r.current);toast((p.name||p.hex)+" = "+fmt(p.code,r.current))}else toast(r.error)};
c.appendChild(inp);c.appendChild(go);d.appendChild(c)}return d}
$("gear").onclick=async()=>{const s=$("props");s.classList.add("on");const box=$("pl");box.textContent="Lecture des réglages…";
try{const ps=await api("/api/props");box.textContent="";const w=ps.filter(p=>p.writable),r=ps.filter(p=>!p.writable);
const h=document.createElement("p");h.className="row";h.textContent=w.length+" réglages modifiables, "+r.length+" en lecture seule";box.appendChild(h);
w.concat(r).forEach(p=>box.appendChild(row(p)))}catch(e){box.textContent="Erreur : "+e}};
st();setInterval(st,1000);
</script></body></html>
"##;
