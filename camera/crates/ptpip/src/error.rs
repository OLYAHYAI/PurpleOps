use std::fmt;
use std::io;

use crate::codes;

/// Erreurs du client PTP/IP, avec des messages destinés à l'utilisateur.
#[derive(Debug)]
pub enum Error {
    /// Erreur réseau ou délai dépassé.
    Io(io::Error),
    /// Paquet ou jeu de données inattendu.
    Protocol(String),
    /// L'appareil a refusé la connexion PTP/IP (paquet Init Fail).
    InitFail(u32),
    /// L'opération a abouti à un code de réponse autre que OK.
    Response { op: u16, code: u16 },
    /// Aucun appareil joignable, ou configuration réseau impossible.
    NoCamera(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Code de réponse PTP si l'erreur en porte un.
    pub fn response_code(&self) -> Option<u16> {
        match self {
            Error::Response { code, .. } => Some(*code),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => match e.kind() {
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                    write!(f, "délai dépassé, l'appareil ne répond plus ({e})")
                }
                io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset => {
                    write!(f, "l'appareil a fermé la connexion ({e})")
                }
                _ => write!(f, "erreur réseau : {e}"),
            },
            Error::Protocol(m) => write!(f, "échange PTP/IP inattendu : {m}"),
            Error::InitFail(r) => write!(
                f,
                "l'appareil a refusé la connexion (Init Fail, motif {r:#x} : {})",
                codes::init_fail_name(*r)
            ),
            Error::Response { op, code } => write!(
                f,
                "{} ({op:#06x}) a échoué : {} ({code:#06x})",
                codes::op_name(*op).unwrap_or("opération inconnue"),
                codes::response_name(*code).unwrap_or("code inconnu")
            ),
            Error::NoCamera(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
