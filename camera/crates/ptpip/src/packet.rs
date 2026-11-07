//! Paquets PTP/IP (CIPA DC-X005) : en-tête de 8 octets (longueur, type), petit-boutiste.

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Error, Result};

pub const INIT_COMMAND_REQUEST: u32 = 0x01;
pub const INIT_COMMAND_ACK: u32 = 0x02;
pub const INIT_EVENT_REQUEST: u32 = 0x03;
pub const INIT_EVENT_ACK: u32 = 0x04;
pub const INIT_FAIL: u32 = 0x05;
pub const OPERATION_REQUEST: u32 = 0x06;
pub const OPERATION_RESPONSE: u32 = 0x07;
pub const EVENT: u32 = 0x08;
pub const START_DATA: u32 = 0x09;
pub const DATA: u32 = 0x0A;
pub const CANCEL: u32 = 0x0B;
pub const END_DATA: u32 = 0x0C;
pub const PROBE_REQUEST: u32 = 0x0D;
pub const PROBE_RESPONSE: u32 = 0x0E;

/// Phase de données d'une requête : aucune ou de l'appareil vers nous.
pub const DATA_PHASE_NONE_OR_IN: u32 = 1;
/// Phase de données d'une requête : de nous vers l'appareil.
pub const DATA_PHASE_OUT: u32 = 2;

/// Version 1.0 : majeure dans les 16 bits hauts.
pub const PROTOCOL_VERSION: u32 = 0x0001_0000;

/// Garde-fou contre un en-tête corrompu (une photo de la P900 fait moins de 16 Mo).
pub const MAX_PACKET: u32 = 128 * 1024 * 1024;

/// Affiche les en-têtes de paquets sur la sortie d'erreur (`--trace`).
pub static TRACE: AtomicBool = AtomicBool::new(false);

pub struct RawPacket {
    pub kind: u32,
    pub payload: Vec<u8>,
}

pub fn kind_name(kind: u32) -> &'static str {
    match kind {
        INIT_COMMAND_REQUEST => "InitCommandRequest",
        INIT_COMMAND_ACK => "InitCommandAck",
        INIT_EVENT_REQUEST => "InitEventRequest",
        INIT_EVENT_ACK => "InitEventAck",
        INIT_FAIL => "InitFail",
        OPERATION_REQUEST => "OperationRequest",
        OPERATION_RESPONSE => "OperationResponse",
        EVENT => "Event",
        START_DATA => "StartData",
        DATA => "Data",
        CANCEL => "Cancel",
        END_DATA => "EndData",
        PROBE_REQUEST => "ProbeRequest",
        PROBE_RESPONSE => "ProbeResponse",
        _ => "type inconnu",
    }
}

fn trace(dir: &str, kind: u32, payload: &[u8]) {
    if !TRACE.load(Ordering::Relaxed) {
        return;
    }
    let shown: Vec<String> = payload.iter().take(32).map(|b| format!("{b:02x}")).collect();
    let more = if payload.len() > 32 { " …" } else { "" };
    eprintln!(
        "[ptpip] {dir} {} ({} o) {}{more}",
        kind_name(kind),
        payload.len() + 8,
        shown.join(" ")
    );
}

pub fn read_packet<R: Read>(r: &mut R) -> io::Result<RawPacket> {
    let mut hdr = [0u8; 8];
    r.read_exact(&mut hdr)?;
    let len = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let kind = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
    if !(8..=MAX_PACKET).contains(&len) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("longueur de paquet PTP/IP invalide : {len}"),
        ));
    }
    let mut payload = vec![0u8; (len - 8) as usize];
    r.read_exact(&mut payload)?;
    trace("<-", kind, &payload);
    Ok(RawPacket { kind, payload })
}

pub fn write_packet<W: Write>(w: &mut W, kind: u32, payload: &[u8]) -> io::Result<()> {
    trace("->", kind, payload);
    let len = (payload.len() + 8) as u32;
    let mut buf = Vec::with_capacity(len as usize);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(&kind.to_le_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

fn short() -> Error {
    Error::Protocol("paquet trop court".into())
}

pub fn le_u16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(short)
}

pub fn le_u32(b: &[u8], at: usize) -> Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(short)
}

pub fn le_u64(b: &[u8], at: usize) -> Result<u64> {
    let lo = le_u32(b, at)? as u64;
    let hi = le_u32(b, at + 4)? as u64;
    Ok(lo | (hi << 32))
}

/// Chaîne UTF-16LE terminée par un zéro.
pub fn utf16z(s: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for u in s.encode_utf16() {
        v.extend_from_slice(&u.to_le_bytes());
    }
    v.extend_from_slice(&[0, 0]);
    v
}

/// Lit une chaîne UTF-16LE terminée par un zéro ; renvoie la chaîne et les octets consommés.
pub fn read_utf16z(b: &[u8]) -> (String, usize) {
    let mut units = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        let u = u16::from_le_bytes([b[i], b[i + 1]]);
        i += 2;
        if u == 0 {
            break;
        }
        units.push(u);
    }
    (String::from_utf16_lossy(&units), i)
}

pub fn init_command_request(guid: &[u8; 16], name: &str) -> Vec<u8> {
    let mut p = guid.to_vec();
    p.extend(utf16z(name));
    p.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    p
}

pub struct InitAck {
    pub conn_number: u32,
    pub guid: [u8; 16],
    pub name: String,
    pub version: u32,
}

pub fn parse_init_command_ack(p: &[u8]) -> Result<InitAck> {
    let conn_number = le_u32(p, 0)?;
    let guid: [u8; 16] = p
        .get(4..20)
        .ok_or_else(short)?
        .try_into()
        .map_err(|_| short())?;
    let (name, used) = read_utf16z(&p[20..]);
    let version = le_u32(p, 20 + used).unwrap_or(0);
    Ok(InitAck {
        conn_number,
        guid,
        name,
        version,
    })
}

pub fn init_command_ack(conn_number: u32, guid: &[u8; 16], name: &str) -> Vec<u8> {
    let mut p = conn_number.to_le_bytes().to_vec();
    p.extend_from_slice(guid);
    p.extend(utf16z(name));
    p.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    p
}

pub fn operation_request(phase: u32, code: u16, tid: u32, params: &[u32]) -> Vec<u8> {
    let mut p = Vec::with_capacity(10 + params.len() * 4);
    p.extend_from_slice(&phase.to_le_bytes());
    p.extend_from_slice(&code.to_le_bytes());
    p.extend_from_slice(&tid.to_le_bytes());
    for x in params.iter().take(5) {
        p.extend_from_slice(&x.to_le_bytes());
    }
    p
}

pub struct OpRequest {
    pub phase: u32,
    pub code: u16,
    pub tid: u32,
    pub params: Vec<u32>,
}

pub fn parse_operation_request(p: &[u8]) -> Result<OpRequest> {
    Ok(OpRequest {
        phase: le_u32(p, 0)?,
        code: le_u16(p, 4)?,
        tid: le_u32(p, 6)?,
        params: params_from(p, 10),
    })
}

fn params_from(p: &[u8], at: usize) -> Vec<u32> {
    p.get(at..)
        .unwrap_or(&[])
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Corps commun aux réponses et aux événements : code (u16), transaction (u32), paramètres.
pub fn code_tid_params(code: u16, tid: u32, params: &[u32]) -> Vec<u8> {
    let mut p = Vec::with_capacity(6 + params.len() * 4);
    p.extend_from_slice(&code.to_le_bytes());
    p.extend_from_slice(&tid.to_le_bytes());
    for x in params {
        p.extend_from_slice(&x.to_le_bytes());
    }
    p
}

pub struct CodeTidParams {
    pub code: u16,
    pub tid: u32,
    pub params: Vec<u32>,
}

pub fn parse_code_tid_params(p: &[u8]) -> Result<CodeTidParams> {
    Ok(CodeTidParams {
        code: le_u16(p, 0)?,
        tid: le_u32(p, 2)?,
        params: params_from(p, 6),
    })
}

pub fn start_data(tid: u32, total: u64) -> Vec<u8> {
    let mut p = tid.to_le_bytes().to_vec();
    p.extend_from_slice(&total.to_le_bytes());
    p
}

pub fn parse_start_data(p: &[u8]) -> Result<(u32, u64)> {
    Ok((le_u32(p, 0)?, le_u64(p, 4)?))
}

/// Corps d'un paquet Data ou EndData : numéro de transaction puis octets.
pub fn data_payload(tid: u32, bytes: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(4 + bytes.len());
    p.extend_from_slice(&tid.to_le_bytes());
    p.extend_from_slice(bytes);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_request_layout() {
        let guid = [7u8; 16];
        let p = init_command_request(&guid, "ab");
        assert_eq!(&p[..16], &guid);
        assert_eq!(&p[16..22], &[b'a', 0, b'b', 0, 0, 0]);
        assert_eq!(&p[22..26], &[0x00, 0x00, 0x01, 0x00]);
    }

    #[test]
    fn init_ack_roundtrip() {
        let guid = [3u8; 16];
        let a = parse_init_command_ack(&init_command_ack(5, &guid, "COOLPIX")).unwrap();
        assert_eq!(a.conn_number, 5);
        assert_eq!(a.guid, guid);
        assert_eq!(a.name, "COOLPIX");
        assert_eq!(a.version, PROTOCOL_VERSION);
    }

    #[test]
    fn packet_roundtrip() {
        let mut buf = Vec::new();
        write_packet(&mut buf, OPERATION_REQUEST, &operation_request(1, 0x1001, 9, &[1, 2])).unwrap();
        let p = read_packet(&mut buf.as_slice()).unwrap();
        assert_eq!(p.kind, OPERATION_REQUEST);
        let r = parse_operation_request(&p.payload).unwrap();
        assert_eq!((r.phase, r.code, r.tid, r.params), (1, 0x1001, 9, vec![1, 2]));
    }

    #[test]
    fn rejects_bad_length() {
        let bad = [4u8, 0, 0, 0, 1, 0, 0, 0];
        assert!(read_packet(&mut bad.as_slice()).is_err());
    }
}
