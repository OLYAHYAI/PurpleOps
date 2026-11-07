//! Client ptpip contre le simulateur : poignée de main, propriétés, visée,
//! déclenchement, téléchargement, refus de connexion.

use std::net::Ipv4Addr;
use std::sync::atomic::Ordering;
use std::time::Duration;

use ptpip::codes::*;
use ptpip::dataset::Value;
use ptpip::{Camera, Error, Options};

fn opts(port: u16) -> Options {
    Options {
        host: Some(Ipv4Addr::LOCALHOST),
        port,
        iface: None,
        guid: [0x11; 16],
        name: "test".into(),
        timeout: Duration::from_secs(3),
    }
}

fn camera() -> (Camera, fakecam::Handle) {
    let h = fakecam::spawn(fakecam::Config::default()).unwrap();
    let cam = Camera::open(&opts(h.addr.port())).unwrap();
    (cam, h)
}

#[test]
fn handshake_and_device_info() {
    let (cam, h) = camera();
    assert_eq!(cam.peer_name, "COOLPIX P900 (simulée)");
    assert_eq!(cam.conn_number, 1);
    let info = cam.info.as_ref().unwrap();
    assert_eq!(info.manufacturer, "Nikon Corporation");
    assert!(cam.supports(OC_NIKON_GET_LIVEVIEW_IMG));
    assert!(!cam.supports(OC_DELETE_OBJECT));
    // La sonde de vie envoyée par le simulateur doit être acquittée.
    for _ in 0..50 {
        if h.probe_answered.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.probe_answered.load(Ordering::SeqCst));
    assert!(cam.event_channel_alive());
}

#[test]
fn properties_read_and_write() {
    let (mut cam, _h) = camera();
    let d = cam.prop_desc(DPC_FOCAL_LENGTH).unwrap();
    assert!(d.writable);
    cam.set_prop(DPC_FOCAL_LENGTH, &Value::U32(2400)).unwrap();
    assert_eq!(cam.prop_value(DPC_FOCAL_LENGTH, d.dtype).unwrap(), Value::U32(2400));
    // Hors plage : refus de l'appareil, remonté comme erreur PTP.
    let e = cam.set_prop(DPC_FOCAL_LENGTH, &Value::U32(99_999)).unwrap_err();
    assert_eq!(e.response_code(), Some(RC_INVALID_DEVICE_PROP_VALUE));
    // Lecture seule.
    let e = cam.set_prop(DPC_BATTERY_LEVEL, &Value::U8(1)).unwrap_err();
    assert_eq!(e.response_code(), Some(RC_ACCESS_DENIED));
}

#[test]
fn liveview_frames_are_jpeg() {
    let (mut cam, _h) = camera();
    let e = cam.liveview_frame().unwrap_err();
    assert_eq!(e.response_code(), Some(RC_NIKON_NOT_LIVEVIEW));
    cam.start_liveview().unwrap();
    for _ in 0..3 {
        let f = cam.liveview_frame().unwrap();
        assert_eq!(&f[..3], &[0xFF, 0xD8, 0xFF]);
        assert_eq!(&f[f.len() - 2..], &[0xFF, 0xD9]);
        assert_eq!(f, fakecam::FRAME_JPEG);
    }
    cam.end_liveview().unwrap();
}

#[test]
fn capture_then_download() {
    let (mut cam, _h) = camera();
    let before = cam.object_handles().unwrap();
    cam.capture(false).unwrap();
    cam.wait_ready(Duration::from_secs(2)).unwrap();
    let h = cam.wait_object_added(Duration::from_secs(2)).unwrap().expect("ObjectAdded");
    assert!(!before.contains(&h));
    let info = cam.object_info(h).unwrap();
    assert_eq!(info.filename, format!("DSCN{h:04}.JPG"));
    let mut data = Vec::new();
    cam.get_object(h, &mut data).unwrap();
    assert_eq!(data.len() as u32, info.size);
    assert_eq!(cam.get_thumb(h).unwrap(), data);
}

#[test]
fn storage_listing() {
    let (mut cam, _h) = camera();
    let ids = cam.storage_ids().unwrap();
    assert_eq!(ids.len(), 1);
    let s = cam.storage_info(ids[0]).unwrap();
    assert_eq!(s.description, "Carte SD simulée");
    assert_eq!(cam.object_handles().unwrap(), vec![1, 2]);
}

#[test]
fn rejected_initiator_is_reported() {
    let h = fakecam::spawn(fakecam::Config {
        reject_init: Some(1),
        ..Default::default()
    })
    .unwrap();
    match Camera::connect(&opts(h.addr.port())) {
        Err(Error::InitFail(1)) => {}
        Err(e) => panic!("erreur inattendue : {e}"),
        Ok(_) => panic!("connexion acceptée à tort"),
    }
}

#[test]
fn unreachable_camera_gives_clear_error() {
    // Port fermé sur la boucle locale : refus immédiat.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let e = Camera::connect(&opts(port)).err().expect("échec attendu");
    assert!(e.to_string().contains("aucun appareil ne répond"), "{e}");
}
