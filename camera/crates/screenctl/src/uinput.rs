//! Périphériques d'entrée virtuels via /dev/uinput (root) : un écran tactile
//! multipoint (protocole B) et un clavier.
//!
//! C'est ce qui rend les gestes instantanés : on écrit directement des événements
//! noyau (~0,1 ms) au lieu de lancer la commande `input`, qui démarre une machine
//! Java à chaque appel (~500 ms). Le défilement devient natif (inertie, appui long).
//!
//! Interface « legacy » de uinput (écriture d'un `uinput_user_dev` puis
//! UI_DEV_CREATE) : présente sur tous les noyaux Android, aucune dépendance.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::io::AsRawFd;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;

const BTN_TOUCH: u16 = 0x14a;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const INPUT_PROP_DIRECT: i32 = 0x01;

// _IOW('U', nr, int) = (1 << 30) | (4 << 16) | ('U' << 8) | nr
const UI_SET_EVBIT: i32 = 0x4004_5564;
const UI_SET_KEYBIT: i32 = 0x4004_5565;
const UI_SET_ABSBIT: i32 = 0x4004_5567;
const UI_SET_PROPBIT: i32 = 0x4004_556e;
const UI_DEV_CREATE: i32 = 0x5501;
const UI_DEV_DESTROY: i32 = 0x5502;

const BUS_VIRTUAL: u16 = 0x06;
const ABS_CNT: usize = 0x40;

// Codes clavier Linux utiles (input-event-codes.h).
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_APPSELECT: u16 = 580;

extern "C" {
    fn ioctl(fd: i32, request: i32, ...) -> i32;
}

struct Device {
    f: File,
}

impl Device {
    fn create(name: &str, product: u16, keys: &[u16], abs: &[(u16, i32, i32)], props: &[i32]) -> io::Result<Self> {
        let f = OpenOptions::new().write(true).open("/dev/uinput")?;
        let fd = f.as_raw_fd();
        let set = |req: i32, v: i32| -> io::Result<()> {
            if unsafe { ioctl(fd, req, v) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        };
        set(UI_SET_EVBIT, EV_SYN as i32)?;
        if !keys.is_empty() {
            set(UI_SET_EVBIT, EV_KEY as i32)?;
            for k in keys {
                set(UI_SET_KEYBIT, *k as i32)?;
            }
        }
        if !abs.is_empty() {
            set(UI_SET_EVBIT, EV_ABS as i32)?;
            for (code, _, _) in abs {
                set(UI_SET_ABSBIT, *code as i32)?;
            }
        }
        for p in props {
            set(UI_SET_PROPBIT, *p)?;
        }

        // struct uinput_user_dev : name[80], input_id (4 x u16), u32 ff_effects_max,
        // s32 absmax[64], absmin[64], absfuzz[64], absflat[64]  = 1116 octets.
        let mut dev = Vec::with_capacity(1116);
        let mut n = [0u8; 80];
        let nb = name.as_bytes();
        n[..nb.len().min(79)].copy_from_slice(&nb[..nb.len().min(79)]);
        dev.extend_from_slice(&n);
        for v in [BUS_VIRTUAL, 0x1209, product, 1] {
            dev.extend_from_slice(&v.to_ne_bytes());
        }
        dev.extend_from_slice(&0u32.to_ne_bytes());
        let mut absmax = [0i32; ABS_CNT];
        let mut absmin = [0i32; ABS_CNT];
        for (code, min, max) in abs {
            absmin[*code as usize] = *min;
            absmax[*code as usize] = *max;
        }
        for arr in [absmax, absmin, [0; ABS_CNT], [0; ABS_CNT]] {
            for v in arr {
                dev.extend_from_slice(&v.to_ne_bytes());
            }
        }
        (&f).write_all(&dev)?;
        if unsafe { ioctl(fd, UI_DEV_CREATE) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Device { f })
    }

    /// Écrit une série d'événements suivie d'un SYN_REPORT.
    fn emit(&mut self, evs: &[(u16, u16, i32)]) -> io::Result<()> {
        let mut b = Vec::with_capacity((evs.len() + 1) * 24);
        for &(t, c, v) in evs.iter().chain(std::iter::once(&(EV_SYN, SYN_REPORT, 0))) {
            b.extend_from_slice(&[0u8; 16]); // struct timeval : le noyau horodate
            b.extend_from_slice(&t.to_ne_bytes());
            b.extend_from_slice(&c.to_ne_bytes());
            b.extend_from_slice(&v.to_ne_bytes());
        }
        self.f.write_all(&b)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            ioctl(self.f.as_raw_fd(), UI_DEV_DESTROY);
        }
    }
}

/// Écran tactile virtuel, aux dimensions de l'écran du téléphone.
pub struct Touch {
    dev: Device,
    next_id: i32,
    down: bool,
}

impl Touch {
    pub fn new(w: u32, h: u32) -> io::Result<Self> {
        let abs = [
            (ABS_MT_SLOT, 0, 9),
            (ABS_MT_TRACKING_ID, 0, 65535),
            (ABS_MT_POSITION_X, 0, w as i32 - 1),
            (ABS_MT_POSITION_Y, 0, h as i32 - 1),
        ];
        let dev = Device::create("PurpleOps ecran tactile", 0x5501, &[BTN_TOUCH], &abs, &[INPUT_PROP_DIRECT])?;
        Ok(Touch { dev, next_id: 1, down: false })
    }

    pub fn down(&mut self, x: u32, y: u32) -> io::Result<()> {
        if self.down {
            return self.moved(x, y);
        }
        self.down = true;
        self.next_id = (self.next_id % 65000) + 1;
        self.dev.emit(&[
            (EV_ABS, ABS_MT_SLOT, 0),
            (EV_ABS, ABS_MT_TRACKING_ID, self.next_id),
            (EV_ABS, ABS_MT_POSITION_X, x as i32),
            (EV_ABS, ABS_MT_POSITION_Y, y as i32),
            (EV_KEY, BTN_TOUCH, 1),
        ])
    }

    pub fn moved(&mut self, x: u32, y: u32) -> io::Result<()> {
        if !self.down {
            return Ok(());
        }
        self.dev.emit(&[(EV_ABS, ABS_MT_POSITION_X, x as i32), (EV_ABS, ABS_MT_POSITION_Y, y as i32)])
    }

    pub fn up(&mut self) -> io::Result<()> {
        if !self.down {
            return Ok(());
        }
        self.down = false;
        self.dev.emit(&[(EV_ABS, ABS_MT_TRACKING_ID, -1), (EV_KEY, BTN_TOUCH, 0)])
    }
}

/// Clavier virtuel : toutes les touches standard + Retour / Accueil / Applis récentes.
/// Créé pour la durée d'une session seulement : tant qu'un clavier physique existe,
/// Android masque le clavier à l'écran.
pub struct Keyboard {
    dev: Device,
}

impl Keyboard {
    pub fn new() -> io::Result<Self> {
        let mut keys: Vec<u16> = (1..=248).collect();
        keys.push(KEY_APPSELECT);
        Ok(Keyboard { dev: Device::create("PurpleOps clavier", 0x5502, &keys, &[], &[])? })
    }

    /// Appui + relâchement d'une touche, avec modificateurs (bit0 Maj, bit1 Ctrl, bit2 Alt).
    pub fn tap(&mut self, code: u16, mods: u32) -> io::Result<()> {
        let m: Vec<u16> = [(1, KEY_LEFTSHIFT), (2, KEY_LEFTCTRL), (4, KEY_LEFTALT)]
            .iter()
            .filter(|(bit, _)| mods & bit != 0)
            .map(|(_, k)| *k)
            .collect();
        for k in &m {
            self.dev.emit(&[(EV_KEY, *k, 1)])?;
        }
        self.dev.emit(&[(EV_KEY, code, 1)])?;
        self.dev.emit(&[(EV_KEY, code, 0)])?;
        for k in m.iter().rev() {
            self.dev.emit(&[(EV_KEY, *k, 0)])?;
        }
        Ok(())
    }
}

/// Caractère ASCII -> (code de touche, Maj) pour une disposition US, celle
/// qu'Android applique par défaut à un clavier virtuel.
pub fn us_key(c: char) -> Option<(u16, bool)> {
    const LETTERS: [u16; 26] = [
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44,
    ];
    Some(match c {
        'a'..='z' => (LETTERS[c as usize - 'a' as usize], false),
        'A'..='Z' => (LETTERS[c as usize - 'A' as usize], true),
        '1'..='9' => (c as u16 - '1' as u16 + 2, false),
        '0' => (11, false),
        ' ' => (57, false),
        '\n' | '\r' => (28, false),
        '\t' => (15, false),
        '!' => (2, true),
        '@' => (3, true),
        '#' => (4, true),
        '$' => (5, true),
        '%' => (6, true),
        '^' => (7, true),
        '&' => (8, true),
        '*' => (9, true),
        '(' => (10, true),
        ')' => (11, true),
        '-' => (12, false),
        '_' => (12, true),
        '=' => (13, false),
        '+' => (13, true),
        '[' => (26, false),
        '{' => (26, true),
        ']' => (27, false),
        '}' => (27, true),
        ';' => (39, false),
        ':' => (39, true),
        '\'' => (40, false),
        '"' => (40, true),
        '`' => (41, false),
        '~' => (41, true),
        '\\' => (43, false),
        '|' => (43, true),
        ',' => (51, false),
        '<' => (51, true),
        '.' => (52, false),
        '>' => (52, true),
        '/' => (53, false),
        '?' => (53, true),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::us_key;

    #[test]
    fn us_map() {
        assert_eq!(us_key('q'), Some((16, false)));
        assert_eq!(us_key('A'), Some((30, true)));
        assert_eq!(us_key('m'), Some((50, false)));
        assert_eq!(us_key('0'), Some((11, false)));
        assert_eq!(us_key('@'), Some((3, true)));
        assert_eq!(us_key('é'), None);
    }
}
