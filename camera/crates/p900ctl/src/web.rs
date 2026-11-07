//! Interface locale : visée MJPEG et boutons dans le navigateur du téléphone.
//!
//! Écoute par défaut sur 127.0.0.1 : le navigateur Android et le chroot partagent
//! le même réseau, rien n'est exposé ailleurs. Chaque requête porte un jeton
//! aléatoire, pour qu'une autre page ouverte dans le navigateur ne puisse pas
//! déclencher l'appareil.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use ptpip::codes::*;
use ptpip::Camera;

use crate::{capture_and_find, download, hex, random_bytes, zoom_target};

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
                "{{\"model\":{},\"liveview\":{},\"zoom\":{},\"fps\":{:.1},\"frames\":{},\"last_file\":{},\"last_error\":{}}}",
                json_str(&sh.model), sh.liveview_ok, sh.zoom_ok, f.fps, f.seq,
                json_str(&st.last_file), json_str(&st.last_error)
            );
            drop((f, st));
            respond(&mut s, "200 OK", "application/json", body.as_bytes())
        }
        ("POST", "/api/capture") => {
            let r = {
                let mut cam = lock(&sh.cam);
                capture_and_find(&mut cam, false).and_then(|(h, dt)| {
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
        ("POST", "/api/zoom") => {
            let dir = if query_param(query, "dir") == Some("in") { 1 } else { -1 };
            let r: Result<String, String> = if !sh.zoom_ok {
                Err("zoom indisponible sur cet appareil".into())
            } else {
                let mut cam = lock(&sh.cam);
                cam.prop_desc(DPC_FOCAL_LENGTH)
                    .map_err(|x| x.to_string())
                    .and_then(|d| {
                        let v = zoom_target(&d, dir).ok_or("limite atteinte")?;
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
        _ => respond(&mut s, "404 Not Found", "text/plain", b""),
    }
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
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>p900ctl</title>
<style>
:root{--bg:#f4f3ef;--fg:#1b1b19;--mute:#6b6a64;--line:#d6d4cc;--acc:#c8102e;--btn:#fff}
@media (prefers-color-scheme:dark){:root{--bg:#121211;--fg:#ecebe6;--mute:#9a988f;--line:#2e2d2a;--acc:#ff4d5e;--btn:#1d1c1a}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.4 system-ui,sans-serif}
header{display:flex;justify-content:space-between;align-items:baseline;padding:12px 16px}
h1{font-size:15px;margin:0;font-weight:600}#st{color:var(--mute);font-size:13px;font-variant-numeric:tabular-nums}
#view{background:#000;aspect-ratio:4/3;display:flex;align-items:center;justify-content:center;color:#888}
#view img{width:100%;height:100%;object-fit:contain}
.bar{display:grid;grid-template-columns:1fr 1.4fr 1fr;gap:12px;padding:16px}
button{font:inherit;font-size:17px;padding:16px 0;border-radius:12px;border:1px solid var(--line);background:var(--btn);color:var(--fg)}
button:disabled{opacity:.35}#shoot{background:var(--acc);border-color:var(--acc);color:#fff;font-weight:600}
#msg{padding:0 16px;min-height:1.4em;color:var(--mute);font-size:13px}
#last{display:block;max-width:40%;margin:12px 16px;border-radius:8px}
</style></head><body>
<header><h1>__MODEL__</h1><span id="st">…</span></header>
<div id="view"><img id="lv" alt="Visée"></div>
<div class="bar"><button id="zout">W −</button><button id="shoot">Déclencher</button><button id="zin">T +</button></div>
<p id="msg"></p><img id="last" hidden alt="Dernière photo">
<script>
const T="__TOKEN__",q=p=>p+(p.includes("?")?"&":"?")+"t="+T,$=id=>document.getElementById(id);
async function st(){try{const s=await (await fetch(q("/api/status"))).json();
$("st").textContent=s.liveview?s.fps.toFixed(1)+" i/s":"visée non disponible";
$("zin").disabled=$("zout").disabled=!s.zoom;if(s.last_error)$("msg").textContent=s.last_error;
if(s.liveview&&!$("lv").src)$("lv").src=q("/stream.mjpg");}catch(e){$("st").textContent="hors ligne"}}
async function post(p,busy){$("msg").textContent=busy;const r=await (await fetch(q(p),{method:"POST"})).json();
$("msg").textContent=r.ok?(r.file?"Enregistrée : "+r.file+" ("+r.ms+" ms)":"Focale : "+r.focal):r.error;return r}
$("shoot").onclick=async()=>{$("shoot").disabled=true;const r=await post("/api/capture","Prise de vue…");
if(r.ok){$("last").src=q("/last.jpg?"+Date.now());$("last").hidden=false}$("shoot").disabled=false;
$("lv").src="";setTimeout(()=>{$("lv").src=q("/stream.mjpg")},300)};
$("zin").onclick=()=>post("/api/zoom?dir=in","Zoom…");$("zout").onclick=()=>post("/api/zoom?dir=out","Zoom…");
st();setInterval(st,1000);
</script></body></html>
"##;
