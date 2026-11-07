//! Simulateur PTP/IP d'une COOLPIX Wi-Fi.
//!
//! Il imite le comportement attendu d'après la norme et libgphoto2, pas celui,
//! encore inconnu, de la vraie P900 : il sert à tester le client, pas à prouver
//! que l'appareil réel répondra de la même façon.

use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use ptpip::codes::*;
use ptpip::dataset::{self, DeviceInfo, Form, ObjectInfo, PropDesc, StorageInfo, Value};
use ptpip::packet::{self, *};

pub const FRAME_JPEG: &[u8] = include_bytes!("../assets/frame.jpg");
const STORAGE_ID: u32 = 0x0001_0001;

#[derive(Clone, Debug)]
pub struct Config {
    pub model: String,
    /// Refuse la poignée de main avec ce motif (test des messages d'erreur).
    pub reject_init: Option<u32>,
    /// Envoie une sonde de vie sur le canal d'événements après la connexion.
    pub send_probe: bool,
    /// Nombre de réponses « occupé » à DeviceReady après le lancement de la visée.
    pub busy_polls: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            model: "COOLPIX P900 (simulée)".into(),
            reject_init: None,
            send_probe: true,
            busy_polls: 2,
        }
    }
}

pub struct Handle {
    pub addr: SocketAddr,
    pub probe_answered: Arc<AtomicBool>,
}

/// Code de réponse, paramètres, données à renvoyer, événement à émettre.
type Reply = (u16, Vec<u32>, Option<Vec<u8>>, Option<(u16, u32)>);

struct FakeObject {
    info: ObjectInfo,
    bytes: Vec<u8>,
}

struct State {
    cfg: Config,
    session: bool,
    liveview: bool,
    busy_left: u32,
    props: HashMap<u16, PropDesc>,
    objects: Vec<(u32, FakeObject)>,
    next_handle: u32,
    nikon_queue: Vec<(u16, u32)>,
}

fn object(n: u32, bytes: &[u8]) -> FakeObject {
    FakeObject {
        info: ObjectInfo {
            storage_id: STORAGE_ID,
            format: 0x3801,
            size: bytes.len() as u32,
            thumb_format: 0x3808,
            thumb_size: bytes.len() as u32,
            thumb_width: 320,
            thumb_height: 240,
            width: 320,
            height: 240,
            bit_depth: 24,
            filename: format!("DSCN{n:04}.JPG"),
            capture_date: "20261107T210000".into(),
            ..Default::default()
        },
        bytes: bytes.to_vec(),
    }
}

impl State {
    fn new(cfg: Config) -> State {
        let mut props = HashMap::new();
        props.insert(
            DPC_BATTERY_LEVEL,
            PropDesc {
                code: DPC_BATTERY_LEVEL,
                dtype: dataset::DT_UINT8,
                writable: false,
                factory: Value::U8(100),
                current: Value::U8(80),
                form: Form::Range {
                    min: Value::U8(0),
                    max: Value::U8(100),
                    step: Value::U8(1),
                },
            },
        );
        props.insert(
            DPC_FOCAL_LENGTH,
            PropDesc {
                code: DPC_FOCAL_LENGTH,
                dtype: dataset::DT_UINT32,
                writable: true,
                factory: Value::U32(430),
                current: Value::U32(430),
                form: Form::Range {
                    min: Value::U32(430),
                    max: Value::U32(35700),
                    step: Value::U32(1),
                },
            },
        );
        props.insert(
            DPC_EXPOSURE_PROGRAM,
            PropDesc {
                code: DPC_EXPOSURE_PROGRAM,
                dtype: dataset::DT_UINT16,
                writable: false,
                factory: Value::U16(2),
                current: Value::U16(2),
                form: Form::Enum(vec![Value::U16(1), Value::U16(2), Value::U16(3), Value::U16(4)]),
            },
        );
        State {
            cfg,
            session: false,
            liveview: false,
            busy_left: 0,
            props,
            objects: vec![(1, object(1, FRAME_JPEG)), (2, object(2, FRAME_JPEG))],
            next_handle: 3,
            nikon_queue: Vec::new(),
        }
    }

    fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            standard_version: 100,
            vendor_ext_id: 0x0000_000A,
            vendor_ext_version: 100,
            vendor_ext_desc: "microsoft.com: 1.0;".into(),
            functional_mode: 0,
            operations: vec![
                OC_GET_DEVICE_INFO,
                OC_OPEN_SESSION,
                OC_CLOSE_SESSION,
                OC_GET_STORAGE_IDS,
                OC_GET_STORAGE_INFO,
                OC_GET_OBJECT_HANDLES,
                OC_GET_OBJECT_INFO,
                OC_GET_OBJECT,
                OC_GET_THUMB,
                OC_GET_DEVICE_PROP_DESC,
                OC_GET_DEVICE_PROP_VALUE,
                OC_SET_DEVICE_PROP_VALUE,
                OC_NIKON_CHECK_EVENT,
                OC_NIKON_DEVICE_READY,
                OC_NIKON_GET_VENDOR_PROP_CODES,
                OC_NIKON_START_LIVEVIEW,
                OC_NIKON_END_LIVEVIEW,
                OC_NIKON_GET_LIVEVIEW_IMG,
                OC_NIKON_INITIATE_CAPTURE_REC_IN_MEDIA,
            ],
            events: vec![EC_OBJECT_ADDED, EC_DEVICE_PROP_CHANGED, EC_CAPTURE_COMPLETE],
            properties: vec![DPC_BATTERY_LEVEL, DPC_FOCAL_LENGTH, DPC_EXPOSURE_PROGRAM],
            capture_formats: vec![0x3801],
            image_formats: vec![0x3801, 0x3808],
            manufacturer: "Nikon Corporation".into(),
            model: self.cfg.model.clone(),
            device_version: "SIM 0.1".into(),
            serial: "00009999".into(),
        }
    }

    fn find(&self, h: u32) -> Option<&FakeObject> {
        self.objects.iter().find(|(k, _)| *k == h).map(|(_, o)| o)
    }

    /// Renvoie (code de réponse, paramètres, données, événement à émettre).
    fn handle(
        &mut self,
        code: u16,
        params: &[u32],
        data_in: Option<&[u8]>,
    ) -> Reply {
        let p0 = params.first().copied().unwrap_or(0);
        let ok = |d: Option<Vec<u8>>| (RC_OK, vec![], d, None);
        match code {
            OC_OPEN_SESSION if self.session => (RC_SESSION_ALREADY_OPEN, vec![], None, None),
            OC_OPEN_SESSION => {
                self.session = true;
                ok(None)
            }
            OC_CLOSE_SESSION => {
                self.session = false;
                ok(None)
            }
            OC_GET_DEVICE_INFO => ok(Some(dataset::encode_device_info(&self.device_info()))),
            _ if !self.session => (0x2003, vec![], None, None),
            OC_GET_STORAGE_IDS => {
                let mut b = Vec::new();
                dataset::put_array_u32(&mut b, &[STORAGE_ID]);
                ok(Some(b))
            }
            OC_GET_STORAGE_INFO => ok(Some(dataset::encode_storage_info(&StorageInfo {
                storage_type: 4,
                filesystem_type: 2,
                access: 0,
                max_capacity: 32 << 30,
                free_bytes: 30 << 30,
                free_images: 4000,
                description: "Carte SD simulée".into(),
                volume_label: "NIKON".into(),
            }))),
            OC_GET_OBJECT_HANDLES => {
                let hs: Vec<u32> = self.objects.iter().map(|(h, _)| *h).collect();
                let mut b = Vec::new();
                dataset::put_array_u32(&mut b, &hs);
                ok(Some(b))
            }
            OC_GET_OBJECT_INFO => match self.find(p0) {
                Some(o) => ok(Some(dataset::encode_object_info(&o.info))),
                None => (RC_INVALID_OBJECT_HANDLE, vec![], None, None),
            },
            OC_GET_OBJECT | OC_GET_THUMB => match self.find(p0) {
                Some(o) => ok(Some(o.bytes.clone())),
                None => (RC_INVALID_OBJECT_HANDLE, vec![], None, None),
            },
            OC_GET_DEVICE_PROP_DESC => match self.props.get(&(p0 as u16)) {
                Some(d) => ok(Some(dataset::encode_prop_desc(d))),
                None => (RC_DEVICE_PROP_NOT_SUPPORTED, vec![], None, None),
            },
            OC_GET_DEVICE_PROP_VALUE => match self.props.get(&(p0 as u16)) {
                Some(d) => {
                    let mut b = Vec::new();
                    dataset::write_value(&d.current, &mut b);
                    ok(Some(b))
                }
                None => (RC_DEVICE_PROP_NOT_SUPPORTED, vec![], None, None),
            },
            OC_SET_DEVICE_PROP_VALUE => self.set_prop(p0 as u16, data_in.unwrap_or(&[])),
            OC_NIKON_GET_VENDOR_PROP_CODES => {
                let mut b = Vec::new();
                dataset::put_array_u16(&mut b, &[]);
                ok(Some(b))
            }
            OC_NIKON_START_LIVEVIEW => {
                self.liveview = true;
                self.busy_left = self.cfg.busy_polls;
                ok(None)
            }
            OC_NIKON_END_LIVEVIEW => {
                self.liveview = false;
                ok(None)
            }
            OC_NIKON_DEVICE_READY if self.busy_left > 0 => {
                self.busy_left -= 1;
                (RC_DEVICE_BUSY, vec![], None, None)
            }
            OC_NIKON_DEVICE_READY => ok(None),
            OC_NIKON_GET_LIVEVIEW_IMG if !self.liveview => (RC_NIKON_NOT_LIVEVIEW, vec![], None, None),
            OC_NIKON_GET_LIVEVIEW_IMG => {
                // En-tête de 128 octets (taille arbitraire, variable selon les modèles Nikon), puis JPEG.
                let mut b = vec![0u8; 128];
                b.extend_from_slice(FRAME_JPEG);
                ok(Some(b))
            }
            OC_NIKON_INITIATE_CAPTURE_REC_IN_MEDIA => {
                let h = self.next_handle;
                self.next_handle += 1;
                self.objects.push((h, object(h, FRAME_JPEG)));
                // Comme un reflex Nikon : la prise de vue interrompt la visée.
                self.liveview = false;
                self.busy_left = self.cfg.busy_polls;
                self.nikon_queue.push((EC_OBJECT_ADDED, h));
                (RC_OK, vec![], None, Some((EC_OBJECT_ADDED, h)))
            }
            OC_NIKON_CHECK_EVENT => {
                let ev = std::mem::take(&mut self.nikon_queue);
                ok(Some(dataset::encode_nikon_events(&ev)))
            }
            _ => (RC_OPERATION_NOT_SUPPORTED, vec![], None, None),
        }
    }

    fn set_prop(
        &mut self,
        code: u16,
        data: &[u8],
    ) -> Reply {
        let Some(d) = self.props.get_mut(&code) else {
            return (RC_DEVICE_PROP_NOT_SUPPORTED, vec![], None, None);
        };
        if !d.writable {
            return (RC_ACCESS_DENIED, vec![], None, None);
        }
        let Ok(v) = dataset::read_value(&mut dataset::Reader::new(data), d.dtype) else {
            return (RC_INVALID_DEVICE_PROP_VALUE, vec![], None, None);
        };
        if let Form::Range { min, max, .. } = &d.form {
            let (n, lo, hi) = (v.as_i128(), min.as_i128(), max.as_i128());
            if n < lo || n > hi {
                return (RC_INVALID_DEVICE_PROP_VALUE, vec![], None, None);
            }
        }
        d.current = v;
        self.nikon_queue.push((EC_DEVICE_PROP_CHANGED, code as u32));
        (RC_OK, vec![], None, Some((EC_DEVICE_PROP_CHANGED, code as u32)))
    }
}

/// Démarre le simulateur sur 127.0.0.1, port libre, dans un fil d'exécution.
pub fn spawn(cfg: Config) -> io::Result<Handle> {
    spawn_on("127.0.0.1:0", cfg)
}

pub fn spawn_on(listen: &str, cfg: Config) -> io::Result<Handle> {
    let listener = TcpListener::bind(listen)?;
    let addr = listener.local_addr()?;
    let probe_answered = Arc::new(AtomicBool::new(false));
    let flag = probe_answered.clone();
    thread::spawn(move || loop {
        let state = State::new(cfg.clone());
        if let Err(e) = serve_one(&listener, state, &flag) {
            if e.kind() != io::ErrorKind::UnexpectedEof {
                eprintln!("[fakecam] session terminée : {e}");
            }
        }
    });
    Ok(Handle {
        addr,
        probe_answered,
    })
}

fn serve_one(listener: &TcpListener, mut st: State, probe_flag: &Arc<AtomicBool>) -> io::Result<()> {
    let (mut cmd, _) = listener.accept()?;
    let p = read_packet(&mut cmd)?;
    if p.kind != INIT_COMMAND_REQUEST {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "InitCommandRequest attendu"));
    }
    if let Some(reason) = st.cfg.reject_init {
        write_packet(&mut cmd, INIT_FAIL, &reason.to_le_bytes())?;
        return Ok(());
    }
    let guid = [0x42u8; 16];
    write_packet(&mut cmd, INIT_COMMAND_ACK, &packet::init_command_ack(1, &guid, &st.cfg.model))?;

    let (mut evt, _) = listener.accept()?;
    let p = read_packet(&mut evt)?;
    if p.kind != INIT_EVENT_REQUEST || packet::le_u32(&p.payload, 0).ok() != Some(1) {
        write_packet(&mut evt, INIT_FAIL, &3u32.to_le_bytes())?;
        return Ok(());
    }
    write_packet(&mut evt, INIT_EVENT_ACK, &[])?;
    let evt_writer = Arc::new(Mutex::new(evt.try_clone()?));

    // Lecteur du canal d'événements : repère la réponse à la sonde de vie.
    {
        let mut rd = evt.try_clone()?;
        let flag = probe_flag.clone();
        thread::spawn(move || {
            while let Ok(p) = read_packet(&mut rd) {
                if p.kind == PROBE_RESPONSE {
                    flag.store(true, Ordering::SeqCst);
                }
            }
        });
    }
    if st.cfg.send_probe {
        let mut w = evt_writer.lock().unwrap();
        write_packet(&mut *w, PROBE_REQUEST, &[])?;
    }

    loop {
        let p = read_packet(&mut cmd)?;
        if p.kind != OPERATION_REQUEST {
            continue;
        }
        let req = packet::parse_operation_request(&p.payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let data_in = if req.phase == DATA_PHASE_OUT {
            Some(read_data_phase(&mut cmd)?)
        } else {
            None
        };
        let (rc, rparams, data_out, event) = st.handle(req.code, &req.params, data_in.as_deref());
        if let Some(d) = data_out {
            send_data(&mut cmd, req.tid, &d)?;
        }
        write_packet(&mut cmd, OPERATION_RESPONSE, &packet::code_tid_params(rc, req.tid, &rparams))?;
        if let Some((code, param)) = event {
            let mut w = evt_writer.lock().unwrap();
            write_packet(&mut *w, EVENT, &packet::code_tid_params(code, 0xFFFF_FFFF, &[param]))?;
        }
        if req.code == OC_CLOSE_SESSION {
            return Ok(());
        }
    }
}

fn read_data_phase(s: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let p = read_packet(s)?;
        match p.kind {
            START_DATA => {}
            DATA => out.extend_from_slice(p.payload.get(4..).unwrap_or(&[])),
            END_DATA => {
                out.extend_from_slice(p.payload.get(4..).unwrap_or(&[]));
                return Ok(out);
            }
            _ => {}
        }
    }
}

/// Envoie les données en trois paquets pour exercer le réassemblage côté client.
fn send_data(s: &mut TcpStream, tid: u32, d: &[u8]) -> io::Result<()> {
    write_packet(s, START_DATA, &packet::start_data(tid, d.len() as u64))?;
    let mid = d.len() / 2;
    write_packet(s, DATA, &packet::data_payload(tid, &d[..mid]))?;
    write_packet(s, END_DATA, &packet::data_payload(tid, &d[mid..]))
}
