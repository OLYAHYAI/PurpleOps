//! Connexion TCP vers l'appareil, avec liaison optionnelle à une interface.
//!
//! Sur Android, un processus lancé dans un chroot ne porte pas la marque réseau
//! que le système pose sur les applications : son trafic suit le réseau par
//! défaut, souvent les données mobiles quand le Wi-Fi de l'appareil photo n'a
//! pas Internet. Lier la socket à `wlan0` (SO_BINDTODEVICE, root requis) force
//! le passage par le Wi-Fi.

use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::process::Command;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

use crate::error::{Error, Result};

pub const DEFAULT_PORT: u16 = 15740;

pub fn connect(addr: SocketAddr, iface: Option<&str>, timeout: Duration) -> Result<TcpStream> {
    let sock = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if let Some(name) = iface {
        bind_to_device(&sock, name)?;
    }
    sock.connect_timeout(&addr.into(), timeout)?;
    let stream: TcpStream = sock.into();
    stream.set_nodelay(true)?;
    Ok(stream)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn bind_to_device(sock: &Socket, name: &str) -> Result<()> {
    sock.bind_device(Some(name.as_bytes())).map_err(|e| {
        if e.kind() == io::ErrorKind::PermissionDenied {
            Error::NoCamera(format!(
                "impossible de lier la socket à {name} : il faut les droits root (CAP_NET_RAW). \
                 Lancez p900ctl en root dans le chroot, ou retirez --iface et coupez les données mobiles."
            ))
        } else {
            Error::NoCamera(format!("impossible de lier la socket à {name} : {e}"))
        }
    })
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn bind_to_device(_sock: &Socket, _name: &str) -> Result<()> {
    Err(Error::NoCamera(
        "--iface n'est pris en charge que sous Linux et Android".into(),
    ))
}

/// Adresse IPv4 d'une interface, lue avec la commande `ip` (iproute2).
pub fn iface_ipv4(iface: &str) -> Option<(Ipv4Addr, u8)> {
    let out = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", iface])
        .output()
        .ok()?;
    parse_ip_addr_output(&String::from_utf8_lossy(&out.stdout))
}

fn parse_ip_addr_output(text: &str) -> Option<(Ipv4Addr, u8)> {
    let mut toks = text.split_whitespace();
    toks.find(|t| *t == "inet")?;
    let cidr = toks.next()?;
    let (ip, prefix) = cidr.split_once('/')?;
    Some((ip.parse().ok()?, prefix.parse().ok()?))
}

/// Passerelle d'une interface, lue avec `ip route get` (fonctionne aussi avec
/// les tables de routage par réseau d'Android).
pub fn iface_gateway(iface: &str) -> Option<Ipv4Addr> {
    let out = Command::new("ip")
        .args(["-4", "route", "get", "8.8.8.8", "oif", iface])
        .output()
        .ok()?;
    parse_route_get_via(&String::from_utf8_lossy(&out.stdout))
}

fn parse_route_get_via(text: &str) -> Option<Ipv4Addr> {
    let mut toks = text.split_whitespace();
    toks.find(|t| *t == "via")?;
    toks.next()?.parse().ok()
}

/// Adresses candidates pour l'appareil photo, dans l'ordre d'essai.
///
/// L'appareil est le point d'accès et le serveur DHCP, donc la passerelle du
/// Wi-Fi. Mesuré sur une P900 : 192.168.0.10 (téléphone en 192.168.0.11).
/// Les autres valeurs couvrent d'autres COOLPIX (192.168.1.1 sur l'A1000).
pub fn guess_hosts(iface: Option<&str>) -> Vec<Ipv4Addr> {
    let name = iface.unwrap_or("wlan0");
    let mut v = Vec::new();
    if let Some(gw) = iface_gateway(name) {
        v.push(gw);
    }
    if let Some((ip, _)) = iface_ipv4(name) {
        let o = ip.octets();
        for last in [10, 1] {
            let c = Ipv4Addr::new(o[0], o[1], o[2], last);
            if !v.contains(&c) {
                v.push(c);
            }
        }
    }
    for d in [
        Ipv4Addr::new(192, 168, 0, 10),
        Ipv4Addr::new(192, 168, 1, 1),
        Ipv4Addr::new(192, 168, 0, 1),
    ] {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ip_output() {
        let line = "23: wlan0    inet 192.168.1.33/24 brd 192.168.1.255 scope global wlan0\\       valid_lft forever";
        assert_eq!(
            parse_ip_addr_output(line),
            Some((Ipv4Addr::new(192, 168, 1, 33), 24))
        );
        assert_eq!(parse_ip_addr_output(""), None);
        // Sortie réelle relevée dans un chroot LineageOS connecté à une P900.
        let route = "192.168.1.1 via 192.168.0.10 dev wlan0 table 1046 src 192.168.0.11 uid 0\n    cache";
        assert_eq!(parse_route_get_via(route), Some(Ipv4Addr::new(192, 168, 0, 10)));
        assert_eq!(parse_route_get_via("8.8.8.8 dev wlan0 src 10.0.0.2"), None);
    }
}
