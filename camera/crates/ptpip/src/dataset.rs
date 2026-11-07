//! Jeux de données PTP (ISO 15740) : lecture, écriture et types de valeurs.

use std::fmt;

use crate::error::{Error, Result};

pub const DT_INT8: u16 = 0x0001;
pub const DT_UINT8: u16 = 0x0002;
pub const DT_INT16: u16 = 0x0003;
pub const DT_UINT16: u16 = 0x0004;
pub const DT_INT32: u16 = 0x0005;
pub const DT_UINT32: u16 = 0x0006;
pub const DT_INT64: u16 = 0x0007;
pub const DT_UINT64: u16 = 0x0008;
pub const DT_INT128: u16 = 0x0009;
pub const DT_UINT128: u16 = 0x000A;
pub const DT_ARRAY: u16 = 0x4000;
pub const DT_STR: u16 = 0xFFFF;

/// Lecteur séquentiel d'un jeu de données petit-boutiste.
pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos + n).ok_or_else(|| {
            Error::Protocol(format!("jeu de données tronqué à l'octet {}", self.pos))
        })?;
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }
    pub fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn u64(&mut self) -> Result<u64> {
        let lo = self.u32()? as u64;
        let hi = self.u32()? as u64;
        Ok(lo | (hi << 32))
    }
    fn bytes16(&mut self) -> Result<[u8; 16]> {
        let mut a = [0u8; 16];
        a.copy_from_slice(self.take(16)?);
        Ok(a)
    }

    /// Chaîne PTP : nombre de caractères sur un octet (zéro final compris), puis UTF-16LE.
    pub fn string(&mut self) -> Result<String> {
        let n = self.u8()? as usize;
        if n == 0 {
            return Ok(String::new());
        }
        let raw = self.take(n * 2)?;
        let units: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }

    fn count(&mut self) -> Result<usize> {
        let n = self.u32()? as usize;
        if n > self.remaining() {
            return Err(Error::Protocol(format!(
                "tableau annoncé de {n} éléments, {} octets restants",
                self.remaining()
            )));
        }
        Ok(n)
    }

    pub fn array_u16(&mut self) -> Result<Vec<u16>> {
        let n = self.count()?;
        (0..n).map(|_| self.u16()).collect()
    }

    pub fn array_u32(&mut self) -> Result<Vec<u32>> {
        let n = self.count()?;
        (0..n).map(|_| self.u32()).collect()
    }
}

pub fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
pub fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

pub fn put_string(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().take(254).collect();
    if units.is_empty() {
        out.push(0);
        return;
    }
    out.push((units.len() + 1) as u8);
    for u in units {
        put_u16(out, u);
    }
    put_u16(out, 0);
}

pub fn put_array_u16(out: &mut Vec<u8>, a: &[u16]) {
    put_u32(out, a.len() as u32);
    for &x in a {
        put_u16(out, x);
    }
}

pub fn put_array_u32(out: &mut Vec<u8>, a: &[u32]) {
    put_u32(out, a.len() as u32);
    for &x in a {
        put_u32(out, x);
    }
}

/// Valeur typée d'une propriété PTP.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    I128([u8; 16]),
    U128([u8; 16]),
    Array(Vec<Value>),
    Str(String),
}

impl Value {
    pub fn as_i128(&self) -> Option<i128> {
        Some(match *self {
            Value::I8(x) => x as i128,
            Value::U8(x) => x as i128,
            Value::I16(x) => x as i128,
            Value::U16(x) => x as i128,
            Value::I32(x) => x as i128,
            Value::U32(x) => x as i128,
            Value::I64(x) => x as i128,
            Value::U64(x) => x as i128,
            _ => return None,
        })
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::I128(b) | Value::U128(b) => {
                f.write_str("0x")?;
                for x in b.iter().rev() {
                    write!(f, "{x:02x}")?;
                }
                Ok(())
            }
            Value::Array(a) => {
                f.write_str("[")?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_str("]")
            }
            Value::Str(s) => write!(f, "« {s} »"),
            v => write!(f, "{}", v.as_i128().unwrap_or_default()),
        }
    }
}

pub fn read_value(r: &mut Reader<'_>, dtype: u16) -> Result<Value> {
    Ok(match dtype {
        DT_INT8 => Value::I8(r.u8()? as i8),
        DT_UINT8 => Value::U8(r.u8()?),
        DT_INT16 => Value::I16(r.u16()? as i16),
        DT_UINT16 => Value::U16(r.u16()?),
        DT_INT32 => Value::I32(r.u32()? as i32),
        DT_UINT32 => Value::U32(r.u32()?),
        DT_INT64 => Value::I64(r.u64()? as i64),
        DT_UINT64 => Value::U64(r.u64()?),
        DT_INT128 => Value::I128(r.bytes16()?),
        DT_UINT128 => Value::U128(r.bytes16()?),
        DT_STR => Value::Str(r.string()?),
        t if t & DT_ARRAY != 0 && (DT_INT8..=DT_UINT128).contains(&(t & 0x00FF)) => {
            let n = r.count()?;
            let mut v = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                v.push(read_value(r, t & 0x00FF)?);
            }
            Value::Array(v)
        }
        t => return Err(Error::Protocol(format!("type de donnée PTP inconnu {t:#06x}"))),
    })
}

pub fn write_value(v: &Value, out: &mut Vec<u8>) {
    match v {
        Value::I8(x) => out.push(*x as u8),
        Value::U8(x) => out.push(*x),
        Value::I16(x) => put_u16(out, *x as u16),
        Value::U16(x) => put_u16(out, *x),
        Value::I32(x) => put_u32(out, *x as u32),
        Value::U32(x) => put_u32(out, *x),
        Value::I64(x) => put_u64(out, *x as u64),
        Value::U64(x) => put_u64(out, *x),
        Value::I128(b) | Value::U128(b) => out.extend_from_slice(b),
        Value::Str(s) => put_string(out, s),
        Value::Array(a) => {
            put_u32(out, a.len() as u32);
            for x in a {
                write_value(x, out);
            }
        }
    }
}

pub fn dtype_name(t: u16) -> String {
    let base = match t & 0x00FF {
        0x01 => "int8",
        0x02 => "uint8",
        0x03 => "int16",
        0x04 => "uint16",
        0x05 => "int32",
        0x06 => "uint32",
        0x07 => "int64",
        0x08 => "uint64",
        0x09 => "int128",
        0x0A => "uint128",
        _ => "?",
    };
    if t == DT_STR {
        "chaîne".into()
    } else if t & DT_ARRAY != 0 {
        format!("tableau<{base}>")
    } else {
        base.into()
    }
}

fn parse_int(s: &str) -> std::result::Result<i128, String> {
    let s = s.trim();
    let (neg, body) = match s.strip_prefix('-') {
        Some(b) => (true, b),
        None => (false, s),
    };
    let v = match body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        Some(h) => i128::from_str_radix(h, 16),
        None => body.parse::<i128>(),
    }
    .map_err(|_| format!("« {s} » n'est pas un nombre"))?;
    Ok(if neg { -v } else { v })
}

/// Convertit un texte saisi en valeur du type attendu par la propriété.
pub fn parse_value(dtype: u16, s: &str) -> std::result::Result<Value, String> {
    if dtype == DT_STR {
        return Ok(Value::Str(s.to_string()));
    }
    let n = parse_int(s)?;
    let out_of_range = || format!("{n} hors des bornes du type {}", dtype_name(dtype));
    Ok(match dtype {
        DT_INT8 => Value::I8(i8::try_from(n).map_err(|_| out_of_range())?),
        DT_UINT8 => Value::U8(u8::try_from(n).map_err(|_| out_of_range())?),
        DT_INT16 => Value::I16(i16::try_from(n).map_err(|_| out_of_range())?),
        DT_UINT16 => Value::U16(u16::try_from(n).map_err(|_| out_of_range())?),
        DT_INT32 => Value::I32(i32::try_from(n).map_err(|_| out_of_range())?),
        DT_UINT32 => Value::U32(u32::try_from(n).map_err(|_| out_of_range())?),
        DT_INT64 => Value::I64(i64::try_from(n).map_err(|_| out_of_range())?),
        DT_UINT64 => Value::U64(u64::try_from(n).map_err(|_| out_of_range())?),
        t => return Err(format!("saisie non prise en charge pour le type {}", dtype_name(t))),
    })
}

/// Construit une valeur entière du même type qu'une valeur modèle.
pub fn int_like(model: &Value, n: i128) -> Option<Value> {
    Some(match model {
        Value::I8(_) => Value::I8(i8::try_from(n).ok()?),
        Value::U8(_) => Value::U8(u8::try_from(n).ok()?),
        Value::I16(_) => Value::I16(i16::try_from(n).ok()?),
        Value::U16(_) => Value::U16(u16::try_from(n).ok()?),
        Value::I32(_) => Value::I32(i32::try_from(n).ok()?),
        Value::U32(_) => Value::U32(u32::try_from(n).ok()?),
        Value::I64(_) => Value::I64(i64::try_from(n).ok()?),
        Value::U64(_) => Value::U64(u64::try_from(n).ok()?),
        _ => return None,
    })
}

// --- DevicePropDesc -----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Form {
    None,
    Range { min: Value, max: Value, step: Value },
    Enum(Vec<Value>),
    Other(u8),
}

#[derive(Clone, Debug)]
pub struct PropDesc {
    pub code: u16,
    pub dtype: u16,
    pub writable: bool,
    pub factory: Value,
    pub current: Value,
    pub form: Form,
}

pub fn parse_prop_desc(b: &[u8]) -> Result<PropDesc> {
    let mut r = Reader::new(b);
    let code = r.u16()?;
    let dtype = r.u16()?;
    let writable = r.u8()? != 0;
    let factory = read_value(&mut r, dtype)?;
    let current = read_value(&mut r, dtype)?;
    let form = if r.remaining() == 0 {
        Form::None
    } else {
        match r.u8()? {
            0 => Form::None,
            1 => Form::Range {
                min: read_value(&mut r, dtype)?,
                max: read_value(&mut r, dtype)?,
                step: read_value(&mut r, dtype)?,
            },
            2 => {
                let n = r.u16()? as usize;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(read_value(&mut r, dtype)?);
                }
                Form::Enum(v)
            }
            f => Form::Other(f),
        }
    };
    Ok(PropDesc {
        code,
        dtype,
        writable,
        factory,
        current,
        form,
    })
}

pub fn encode_prop_desc(d: &PropDesc) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, d.code);
    put_u16(&mut out, d.dtype);
    out.push(d.writable as u8);
    write_value(&d.factory, &mut out);
    write_value(&d.current, &mut out);
    match &d.form {
        Form::None => out.push(0),
        Form::Range { min, max, step } => {
            out.push(1);
            write_value(min, &mut out);
            write_value(max, &mut out);
            write_value(step, &mut out);
        }
        Form::Enum(v) => {
            out.push(2);
            put_u16(&mut out, v.len() as u16);
            for x in v {
                write_value(x, &mut out);
            }
        }
        Form::Other(f) => out.push(*f),
    }
    out
}

// --- DeviceInfo ---------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DeviceInfo {
    pub standard_version: u16,
    pub vendor_ext_id: u32,
    pub vendor_ext_version: u16,
    pub vendor_ext_desc: String,
    pub functional_mode: u16,
    pub operations: Vec<u16>,
    pub events: Vec<u16>,
    pub properties: Vec<u16>,
    pub capture_formats: Vec<u16>,
    pub image_formats: Vec<u16>,
    pub manufacturer: String,
    pub model: String,
    pub device_version: String,
    pub serial: String,
}

pub fn parse_device_info(b: &[u8]) -> Result<DeviceInfo> {
    let mut r = Reader::new(b);
    Ok(DeviceInfo {
        standard_version: r.u16()?,
        vendor_ext_id: r.u32()?,
        vendor_ext_version: r.u16()?,
        vendor_ext_desc: r.string()?,
        functional_mode: r.u16()?,
        operations: r.array_u16()?,
        events: r.array_u16()?,
        properties: r.array_u16()?,
        capture_formats: r.array_u16()?,
        image_formats: r.array_u16()?,
        manufacturer: r.string()?,
        model: r.string()?,
        device_version: r.string()?,
        serial: r.string()?,
    })
}

pub fn encode_device_info(d: &DeviceInfo) -> Vec<u8> {
    let mut o = Vec::new();
    put_u16(&mut o, d.standard_version);
    put_u32(&mut o, d.vendor_ext_id);
    put_u16(&mut o, d.vendor_ext_version);
    put_string(&mut o, &d.vendor_ext_desc);
    put_u16(&mut o, d.functional_mode);
    put_array_u16(&mut o, &d.operations);
    put_array_u16(&mut o, &d.events);
    put_array_u16(&mut o, &d.properties);
    put_array_u16(&mut o, &d.capture_formats);
    put_array_u16(&mut o, &d.image_formats);
    put_string(&mut o, &d.manufacturer);
    put_string(&mut o, &d.model);
    put_string(&mut o, &d.device_version);
    put_string(&mut o, &d.serial);
    o
}

// --- ObjectInfo ---------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct ObjectInfo {
    pub storage_id: u32,
    pub format: u16,
    pub protection: u16,
    pub size: u32,
    pub thumb_format: u16,
    pub thumb_size: u32,
    pub thumb_width: u32,
    pub thumb_height: u32,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub parent: u32,
    pub association_type: u16,
    pub association_desc: u32,
    pub sequence: u32,
    pub filename: String,
    pub capture_date: String,
    pub modification_date: String,
    pub keywords: String,
}

pub fn parse_object_info(b: &[u8]) -> Result<ObjectInfo> {
    let mut r = Reader::new(b);
    Ok(ObjectInfo {
        storage_id: r.u32()?,
        format: r.u16()?,
        protection: r.u16()?,
        size: r.u32()?,
        thumb_format: r.u16()?,
        thumb_size: r.u32()?,
        thumb_width: r.u32()?,
        thumb_height: r.u32()?,
        width: r.u32()?,
        height: r.u32()?,
        bit_depth: r.u32()?,
        parent: r.u32()?,
        association_type: r.u16()?,
        association_desc: r.u32()?,
        sequence: r.u32()?,
        filename: r.string()?,
        capture_date: r.string().unwrap_or_default(),
        modification_date: r.string().unwrap_or_default(),
        keywords: r.string().unwrap_or_default(),
    })
}

pub fn encode_object_info(o: &ObjectInfo) -> Vec<u8> {
    let mut b = Vec::new();
    put_u32(&mut b, o.storage_id);
    put_u16(&mut b, o.format);
    put_u16(&mut b, o.protection);
    put_u32(&mut b, o.size);
    put_u16(&mut b, o.thumb_format);
    put_u32(&mut b, o.thumb_size);
    put_u32(&mut b, o.thumb_width);
    put_u32(&mut b, o.thumb_height);
    put_u32(&mut b, o.width);
    put_u32(&mut b, o.height);
    put_u32(&mut b, o.bit_depth);
    put_u32(&mut b, o.parent);
    put_u16(&mut b, o.association_type);
    put_u32(&mut b, o.association_desc);
    put_u32(&mut b, o.sequence);
    put_string(&mut b, &o.filename);
    put_string(&mut b, &o.capture_date);
    put_string(&mut b, &o.modification_date);
    put_string(&mut b, &o.keywords);
    b
}

// --- StorageInfo --------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct StorageInfo {
    pub storage_type: u16,
    pub filesystem_type: u16,
    pub access: u16,
    pub max_capacity: u64,
    pub free_bytes: u64,
    pub free_images: u32,
    pub description: String,
    pub volume_label: String,
}

pub fn parse_storage_info(b: &[u8]) -> Result<StorageInfo> {
    let mut r = Reader::new(b);
    Ok(StorageInfo {
        storage_type: r.u16()?,
        filesystem_type: r.u16()?,
        access: r.u16()?,
        max_capacity: r.u64()?,
        free_bytes: r.u64()?,
        free_images: r.u32()?,
        description: r.string().unwrap_or_default(),
        volume_label: r.string().unwrap_or_default(),
    })
}

pub fn encode_storage_info(s: &StorageInfo) -> Vec<u8> {
    let mut b = Vec::new();
    put_u16(&mut b, s.storage_type);
    put_u16(&mut b, s.filesystem_type);
    put_u16(&mut b, s.access);
    put_u64(&mut b, s.max_capacity);
    put_u64(&mut b, s.free_bytes);
    put_u32(&mut b, s.free_images);
    put_string(&mut b, &s.description);
    put_string(&mut b, &s.volume_label);
    b
}

// --- Nikon CheckEvent -----------------------------------------------------------

/// Réponse de Nikon CheckEvent (0x90C7) : u16 nombre, puis (u16 code, u32 paramètre).
pub fn parse_nikon_events(b: &[u8]) -> Result<Vec<(u16, u32)>> {
    if b.is_empty() {
        return Ok(Vec::new());
    }
    let mut r = Reader::new(b);
    let n = r.u16()? as usize;
    let mut v = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        v.push((r.u16()?, r.u32()?));
    }
    Ok(v)
}

pub fn encode_nikon_events(ev: &[(u16, u32)]) -> Vec<u8> {
    let mut b = Vec::new();
    put_u16(&mut b, ev.len() as u16);
    for &(c, p) in ev {
        put_u16(&mut b, c);
        put_u32(&mut b, p);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_info_roundtrip() {
        let d = DeviceInfo {
            standard_version: 100,
            vendor_ext_id: 0x0A,
            vendor_ext_version: 100,
            vendor_ext_desc: "Nikon".into(),
            operations: vec![0x1001, 0x9201],
            properties: vec![0x5008],
            manufacturer: "Nikon Corporation".into(),
            model: "COOLPIX P900".into(),
            device_version: "V1.0".into(),
            serial: "00001234".into(),
            ..Default::default()
        };
        let back = parse_device_info(&encode_device_info(&d)).unwrap();
        assert_eq!(back.model, "COOLPIX P900");
        assert_eq!(back.operations, vec![0x1001, 0x9201]);
        assert_eq!(back.serial, "00001234");
    }

    #[test]
    fn prop_desc_range_roundtrip() {
        let d = PropDesc {
            code: 0x5008,
            dtype: DT_UINT32,
            writable: true,
            factory: Value::U32(430),
            current: Value::U32(1000),
            form: Form::Range {
                min: Value::U32(430),
                max: Value::U32(35700),
                step: Value::U32(1),
            },
        };
        let back = parse_prop_desc(&encode_prop_desc(&d)).unwrap();
        assert!(back.writable);
        assert_eq!(back.current, Value::U32(1000));
        assert_eq!(back.form, d.form);
    }

    #[test]
    fn values_parse_and_format() {
        assert_eq!(parse_value(DT_UINT16, "0x10").unwrap(), Value::U16(16));
        assert_eq!(parse_value(DT_INT8, "-3").unwrap(), Value::I8(-3));
        assert!(parse_value(DT_UINT8, "300").is_err());
        assert_eq!(Value::Str("a".into()).to_string(), "« a »");
        let mut b = Vec::new();
        write_value(&Value::Array(vec![Value::U16(1), Value::U16(2)]), &mut b);
        let v = read_value(&mut Reader::new(&b), DT_ARRAY | DT_UINT16).unwrap();
        assert_eq!(v.to_string(), "[1, 2]");
    }

    #[test]
    fn truncated_array_is_an_error() {
        let b = [0xFF, 0xFF, 0xFF, 0x00, 1, 0];
        assert!(Reader::new(&b).array_u16().is_err());
    }
}
