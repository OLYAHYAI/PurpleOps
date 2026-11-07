//! Session PTP/IP avec un appareil photo : poignée de main, transactions,
//! canal d'événements et opérations de haut niveau.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::codes::*;
use crate::dataset::{self, DeviceInfo, ObjectInfo, PropDesc, StorageInfo, Value};
use crate::error::{Error, Result};
use crate::net;
use crate::packet::{self, *};

/// Paramètres de connexion.
#[derive(Clone, Debug)]
pub struct Options {
    /// Adresse de l'appareil ; `None` = essai des adresses probables.
    pub host: Option<Ipv4Addr>,
    pub port: u16,
    /// Interface à laquelle lier les sockets (ex. `wlan0`), root requis.
    pub iface: Option<String>,
    /// Identifiant de l'initiateur, gardé d'une session à l'autre.
    pub guid: [u8; 16],
    /// Nom présenté à l'appareil.
    pub name: String,
    /// Délai de lecture et d'écriture sur le canal de commandes.
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            host: None,
            port: net::DEFAULT_PORT,
            iface: None,
            guid: [0u8; 16],
            name: "p900ctl".into(),
            timeout: Duration::from_secs(10),
        }
    }
}

/// Événement PTP reçu de l'appareil.
#[derive(Clone, Debug)]
pub struct Event {
    pub code: u16,
    pub params: Vec<u32>,
    pub at: Instant,
}

/// Résultat brut d'une transaction.
pub struct Response {
    pub code: u16,
    pub params: Vec<u32>,
    pub data: Vec<u8>,
}

/// Destination des données reçues pendant une transaction.
pub trait DataSink {
    fn start(&mut self, _total: u64) {}
    fn chunk(&mut self, b: &[u8]) -> io::Result<()>;
}

impl DataSink for Vec<u8> {
    fn start(&mut self, total: u64) {
        self.reserve(total.min(64 << 20) as usize);
    }
    fn chunk(&mut self, b: &[u8]) -> io::Result<()> {
        self.extend_from_slice(b);
        Ok(())
    }
}

/// Extrait le JPEG d'une image de visée Nikon (en-tête propre au modèle, puis JPEG).
pub fn extract_jpeg(b: &[u8]) -> Option<&[u8]> {
    let start = b.windows(3).position(|w| w == [0xFF, 0xD8, 0xFF])?;
    let end = b[start..].windows(2).rposition(|w| w == [0xFF, 0xD9])? + start + 2;
    Some(&b[start..end])
}

pub struct Camera {
    cmd: TcpStream,
    event_ctl: TcpStream,
    events: Receiver<Event>,
    event_alive: Arc<AtomicBool>,
    probes_answered: Arc<AtomicU32>,
    next_tid: u32,
    session_open: bool,
    pub addr: SocketAddr,
    pub peer_name: String,
    pub conn_number: u32,
    pub peer_version: u32,
    pub info: Option<DeviceInfo>,
}

fn init_fail_reason(p: &[u8]) -> u32 {
    packet::le_u32(p, 0).unwrap_or(0)
}

impl Camera {
    /// Ouvre les deux connexions PTP/IP (commandes puis événements).
    /// N'envoie aucune opération PTP : l'appareil n'est pas modifié.
    pub fn connect(opts: &Options) -> Result<Camera> {
        let candidates = match opts.host {
            Some(h) => vec![h],
            None => net::guess_hosts(opts.iface.as_deref()),
        };
        let probe_timeout = if opts.host.is_some() {
            opts.timeout
        } else {
            Duration::from_millis(1500)
        };
        let mut last_err = String::new();
        let mut found = None;
        for ip in &candidates {
            let addr = SocketAddr::new(IpAddr::V4(*ip), opts.port);
            match net::connect(addr, opts.iface.as_deref(), probe_timeout) {
                Ok(s) => {
                    found = Some((addr, s));
                    break;
                }
                Err(e @ Error::NoCamera(_)) => return Err(e),
                Err(e) => last_err = format!("{addr} : {e}"),
            }
        }
        let (addr, mut cmd) = found.ok_or_else(|| {
            let tried: Vec<String> = candidates.iter().map(|i| i.to_string()).collect();
            Error::NoCamera(format!(
                "aucun appareil ne répond sur le port {} (essayé : {}). Dernière erreur : {last_err}. \
                 Vérifiez que le téléphone est connecté au Wi-Fi de l'appareil et que le trafic \
                 passe par le Wi-Fi (--iface wlan0, ou données mobiles coupées).",
                opts.port,
                tried.join(", ")
            ))
        })?;
        cmd.set_read_timeout(Some(opts.timeout))?;
        cmd.set_write_timeout(Some(opts.timeout))?;

        packet::write_packet(
            &mut cmd,
            INIT_COMMAND_REQUEST,
            &packet::init_command_request(&opts.guid, &opts.name),
        )?;
        let p = packet::read_packet(&mut cmd)?;
        let ack = match p.kind {
            INIT_COMMAND_ACK => packet::parse_init_command_ack(&p.payload)?,
            INIT_FAIL => return Err(Error::InitFail(init_fail_reason(&p.payload))),
            k => {
                return Err(Error::Protocol(format!(
                    "attendu InitCommandAck, reçu {}",
                    packet::kind_name(k)
                )))
            }
        };

        let mut evt = net::connect(addr, opts.iface.as_deref(), opts.timeout)?;
        evt.set_read_timeout(Some(opts.timeout))?;
        evt.set_write_timeout(Some(opts.timeout))?;
        packet::write_packet(&mut evt, INIT_EVENT_REQUEST, &ack.conn_number.to_le_bytes())?;
        let p = packet::read_packet(&mut evt)?;
        match p.kind {
            INIT_EVENT_ACK => {}
            INIT_FAIL => return Err(Error::InitFail(init_fail_reason(&p.payload))),
            k => {
                return Err(Error::Protocol(format!(
                    "attendu InitEventAck, reçu {}",
                    packet::kind_name(k)
                )))
            }
        }
        evt.set_read_timeout(None)?;

        let event_ctl = evt.try_clone()?;
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let probes = Arc::new(AtomicU32::new(0));
        {
            let alive = alive.clone();
            let probes = probes.clone();
            thread::Builder::new()
                .name("ptpip-events".into())
                .spawn(move || event_loop(evt, tx, alive, probes))?;
        }

        Ok(Camera {
            cmd,
            event_ctl,
            events: rx,
            event_alive: alive,
            probes_answered: probes,
            next_tid: 1,
            session_open: false,
            addr,
            peer_name: ack.name,
            conn_number: ack.conn_number,
            peer_version: ack.version,
            info: None,
        })
    }

    /// Connexion, ouverture de session et lecture de DeviceInfo.
    pub fn open(opts: &Options) -> Result<Camera> {
        let mut cam = Camera::connect(opts)?;
        cam.open_session()?;
        cam.device_info()?;
        Ok(cam)
    }

    pub fn event_channel_alive(&self) -> bool {
        self.event_alive.load(Ordering::SeqCst)
    }

    /// Nombre de sondes de vie (ProbeRequest) reçues et acquittées.
    pub fn probes_answered(&self) -> u32 {
        self.probes_answered.load(Ordering::SeqCst)
    }

    pub fn set_timeout(&mut self, t: Duration) -> Result<()> {
        self.cmd.set_read_timeout(Some(t))?;
        Ok(())
    }

    fn alloc_tid(&mut self, code: u16) -> u32 {
        // ISO 15740 : OpenSession utilise la transaction 0, les suivantes partent de 1.
        if code == OC_OPEN_SESSION {
            self.next_tid = 1;
            return 0;
        }
        let t = self.next_tid;
        self.next_tid = if t >= 0xFFFF_FFFE { 1 } else { t + 1 };
        t
    }

    /// Transaction générique : requête, données éventuelles, réponse.
    pub fn run(
        &mut self,
        code: u16,
        params: &[u32],
        send: Option<&[u8]>,
        sink: &mut dyn DataSink,
    ) -> Result<(u16, Vec<u32>)> {
        let tid = self.alloc_tid(code);
        let phase = if send.is_some() {
            DATA_PHASE_OUT
        } else {
            DATA_PHASE_NONE_OR_IN
        };
        packet::write_packet(
            &mut self.cmd,
            OPERATION_REQUEST,
            &packet::operation_request(phase, code, tid, params),
        )?;
        if let Some(d) = send {
            packet::write_packet(&mut self.cmd, START_DATA, &packet::start_data(tid, d.len() as u64))?;
            packet::write_packet(&mut self.cmd, END_DATA, &packet::data_payload(tid, d))?;
        }
        loop {
            let p = packet::read_packet(&mut self.cmd)?;
            match p.kind {
                START_DATA => {
                    let (_, total) = packet::parse_start_data(&p.payload)?;
                    sink.start(total);
                }
                DATA | END_DATA => {
                    let body = p
                        .payload
                        .get(4..)
                        .ok_or_else(|| Error::Protocol("paquet de données trop court".into()))?;
                    sink.chunk(body)?;
                }
                OPERATION_RESPONSE => {
                    let r = packet::parse_code_tid_params(&p.payload)?;
                    if r.tid != tid && packet::TRACE.load(Ordering::Relaxed) {
                        eprintln!("[ptpip] réponse à la transaction {} (attendu {tid})", r.tid);
                    }
                    return Ok((r.code, r.params));
                }
                PROBE_REQUEST => packet::write_packet(&mut self.cmd, PROBE_RESPONSE, &[])?,
                CANCEL => return Err(Error::Protocol("transfert annulé par l'appareil".into())),
                _ => {}
            }
        }
    }

    pub fn transaction(&mut self, code: u16, params: &[u32], send: Option<&[u8]>) -> Result<Response> {
        let mut data = Vec::new();
        let (rc, rp) = self.run(code, params, send, &mut data)?;
        Ok(Response {
            code: rc,
            params: rp,
            data,
        })
    }

    /// Transaction dont le code de réponse doit être OK.
    pub fn call(&mut self, code: u16, params: &[u32]) -> Result<Response> {
        let r = self.transaction(code, params, None)?;
        if r.code != RC_OK {
            return Err(Error::Response { op: code, code: r.code });
        }
        Ok(r)
    }

    pub fn supports(&self, op: u16) -> bool {
        self.info.as_ref().is_some_and(|i| i.operations.contains(&op))
    }

    // --- Session et informations ----------------------------------------------

    pub fn open_session(&mut self) -> Result<()> {
        let r = self.transaction(OC_OPEN_SESSION, &[1], None)?;
        match r.code {
            RC_OK | RC_SESSION_ALREADY_OPEN => {
                self.session_open = true;
                Ok(())
            }
            c => Err(Error::Response {
                op: OC_OPEN_SESSION,
                code: c,
            }),
        }
    }

    pub fn close_session(&mut self) -> Result<()> {
        if self.session_open {
            self.session_open = false;
            self.call(OC_CLOSE_SESSION, &[])?;
        }
        Ok(())
    }

    pub fn device_info(&mut self) -> Result<&DeviceInfo> {
        let r = self.call(OC_GET_DEVICE_INFO, &[])?;
        self.info = Some(dataset::parse_device_info(&r.data)?);
        Ok(self.info.as_ref().unwrap())
    }

    /// Propriétés que Nikon n'annonce pas dans DeviceInfo (0x90CA).
    pub fn vendor_prop_codes(&mut self) -> Result<Vec<u16>> {
        let r = self.call(OC_NIKON_GET_VENDOR_PROP_CODES, &[])?;
        dataset::Reader::new(&r.data).array_u16()
    }

    // --- Propriétés ---------------------------------------------------------------

    pub fn prop_desc(&mut self, code: u16) -> Result<PropDesc> {
        let r = self.call(OC_GET_DEVICE_PROP_DESC, &[code as u32])?;
        dataset::parse_prop_desc(&r.data)
    }

    pub fn prop_value(&mut self, code: u16, dtype: u16) -> Result<Value> {
        let r = self.call(OC_GET_DEVICE_PROP_VALUE, &[code as u32])?;
        dataset::read_value(&mut dataset::Reader::new(&r.data), dtype)
    }

    pub fn set_prop(&mut self, code: u16, value: &Value) -> Result<()> {
        let mut data = Vec::new();
        dataset::write_value(value, &mut data);
        let r = self.transaction(OC_SET_DEVICE_PROP_VALUE, &[code as u32], Some(&data))?;
        if r.code != RC_OK {
            return Err(Error::Response {
                op: OC_SET_DEVICE_PROP_VALUE,
                code: r.code,
            });
        }
        Ok(())
    }

    // --- Stockage et objets -----------------------------------------------------

    pub fn storage_ids(&mut self) -> Result<Vec<u32>> {
        let r = self.call(OC_GET_STORAGE_IDS, &[])?;
        dataset::Reader::new(&r.data).array_u32()
    }

    pub fn storage_info(&mut self, id: u32) -> Result<StorageInfo> {
        let r = self.call(OC_GET_STORAGE_INFO, &[id])?;
        dataset::parse_storage_info(&r.data)
    }

    /// Tous les objets de tous les supports.
    pub fn object_handles(&mut self) -> Result<Vec<u32>> {
        let r = self.call(OC_GET_OBJECT_HANDLES, &[0xFFFF_FFFF, 0, 0])?;
        dataset::Reader::new(&r.data).array_u32()
    }

    pub fn object_info(&mut self, handle: u32) -> Result<ObjectInfo> {
        let r = self.call(OC_GET_OBJECT_INFO, &[handle])?;
        dataset::parse_object_info(&r.data)
    }

    pub fn get_object(&mut self, handle: u32, sink: &mut dyn DataSink) -> Result<()> {
        let (rc, _) = self.run(OC_GET_OBJECT, &[handle], None, sink)?;
        if rc != RC_OK {
            return Err(Error::Response {
                op: OC_GET_OBJECT,
                code: rc,
            });
        }
        Ok(())
    }

    pub fn get_thumb(&mut self, handle: u32) -> Result<Vec<u8>> {
        Ok(self.call(OC_GET_THUMB, &[handle])?.data)
    }

    // --- Nikon : état, visée, déclenchement ---------------------------------------

    /// Attend que l'appareil ne réponde plus « occupé » à DeviceReady.
    pub fn wait_ready(&mut self, timeout: Duration) -> Result<()> {
        if !self.supports(OC_NIKON_DEVICE_READY) {
            thread::sleep(Duration::from_millis(300));
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        loop {
            let r = self.transaction(OC_NIKON_DEVICE_READY, &[], None)?;
            match r.code {
                RC_OK => return Ok(()),
                RC_DEVICE_BUSY => {}
                c => {
                    return Err(Error::Response {
                        op: OC_NIKON_DEVICE_READY,
                        code: c,
                    })
                }
            }
            if Instant::now() >= deadline {
                return Err(Error::Protocol("l'appareil reste occupé".into()));
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn start_liveview(&mut self) -> Result<()> {
        let r = self.transaction(OC_NIKON_START_LIVEVIEW, &[], None)?;
        if r.code != RC_OK && r.code != RC_DEVICE_BUSY {
            return Err(Error::Response {
                op: OC_NIKON_START_LIVEVIEW,
                code: r.code,
            });
        }
        self.wait_ready(Duration::from_secs(5))
    }

    pub fn end_liveview(&mut self) -> Result<()> {
        self.call(OC_NIKON_END_LIVEVIEW, &[])?;
        Ok(())
    }

    /// Une image de visée, réduite au JPEG.
    pub fn liveview_frame(&mut self) -> Result<Vec<u8>> {
        let r = self.call(OC_NIKON_GET_LIVEVIEW_IMG, &[])?;
        extract_jpeg(&r.data).map(|s| s.to_vec()).ok_or_else(|| {
            Error::Protocol(format!("image de visée sans JPEG ({} octets)", r.data.len()))
        })
    }

    /// Mise au point automatique à la demande (Nikon AfDrive, 0x90C1).
    pub fn af_drive(&mut self) -> Result<()> {
        self.call(OC_NIKON_AF_DRIVE, &[])?;
        self.wait_ready(Duration::from_secs(5))
    }

    /// Événements en attente par Nikon CheckEvent (0x90C7).
    pub fn check_events(&mut self) -> Result<Vec<(u16, u32)>> {
        let r = self.call(OC_NIKON_CHECK_EVENT, &[])?;
        dataset::parse_nikon_events(&r.data)
    }

    /// Déclenche une prise de vue ; renvoie l'opération utilisée.
    ///
    /// Paramètres de 0x9207 repris de libgphoto2 : 0xFFFFFFFF sans mise au
    /// point préalable, 0xFFFFFFFE avec. Non vérifié sur la P900.
    pub fn capture(&mut self, autofocus: bool) -> Result<u16> {
        if self.supports(OC_NIKON_INITIATE_CAPTURE_REC_IN_MEDIA) {
            let p1 = if autofocus { 0xFFFF_FFFE } else { 0xFFFF_FFFF };
            self.call(OC_NIKON_INITIATE_CAPTURE_REC_IN_MEDIA, &[p1, 0])?;
            Ok(OC_NIKON_INITIATE_CAPTURE_REC_IN_MEDIA)
        } else if self.supports(OC_INITIATE_CAPTURE) {
            self.call(OC_INITIATE_CAPTURE, &[0, 0])?;
            Ok(OC_INITIATE_CAPTURE)
        } else {
            Err(Error::Protocol(
                "l'appareil n'annonce aucune opération de déclenchement".into(),
            ))
        }
    }

    /// Événements reçus depuis le dernier appel : canal d'événements PTP/IP,
    /// puis Nikon CheckEvent si l'appareil le propose.
    pub fn poll_events(&mut self) -> Result<Vec<Event>> {
        let mut out: Vec<Event> = self.events.try_iter().collect();
        if self.supports(OC_NIKON_CHECK_EVENT) {
            let now = Instant::now();
            for (code, param) in self.check_events()? {
                out.push(Event {
                    code,
                    params: vec![param],
                    at: now,
                });
            }
        }
        Ok(out)
    }

    /// Attend l'événement ObjectAdded ; renvoie le handle du nouvel objet.
    pub fn wait_object_added(&mut self, timeout: Duration) -> Result<Option<u32>> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            for ev in self.poll_events()? {
                if ev.code == EC_OBJECT_ADDED {
                    return Ok(ev.params.first().copied());
                }
            }
            thread::sleep(Duration::from_millis(150));
        }
        Ok(None)
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        if self.session_open {
            let _ = self.cmd.set_read_timeout(Some(Duration::from_secs(2)));
            let _ = self.close_session();
        }
        let _ = self.event_ctl.shutdown(Shutdown::Both);
        let _ = self.cmd.shutdown(Shutdown::Both);
    }
}

fn event_loop(
    mut s: TcpStream,
    tx: Sender<Event>,
    alive: Arc<AtomicBool>,
    probes: Arc<AtomicU32>,
) {
    while let Ok(p) = packet::read_packet(&mut s) {
        match p.kind {
            EVENT => {
                if let Ok(e) = packet::parse_code_tid_params(&p.payload) {
                    let _ = tx.send(Event {
                        code: e.code,
                        params: e.params,
                        at: Instant::now(),
                    });
                }
            }
            PROBE_REQUEST => {
                if packet::write_packet(&mut s, PROBE_RESPONSE, &[]).is_err() {
                    break;
                }
                probes.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    }
    alive.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_is_cut_out_of_nikon_frame() {
        let mut frame = vec![0u8; 64];
        frame.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 0xFF, 0xD9]);
        frame.extend_from_slice(&[0, 0, 0]);
        assert_eq!(
            extract_jpeg(&frame).unwrap(),
            &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 0xFF, 0xD9]
        );
        assert!(extract_jpeg(&[1, 2, 3]).is_none());
    }
}
