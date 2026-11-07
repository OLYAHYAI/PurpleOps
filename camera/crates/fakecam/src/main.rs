//! fakecam : simulateur PTP/IP pour essayer p900ctl sans l'appareil.
//!
//!     fakecam [--listen 127.0.0.1:15740] [--reject MOTIF]

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut listen = "127.0.0.1:15740".to_string();
    let mut cfg = fakecam::Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().unwrap_or(listen),
            "--reject" => cfg.reject_init = args.next().and_then(|v| v.parse().ok()),
            "-h" | "--help" => {
                println!("fakecam [--listen ADRESSE:PORT] [--reject MOTIF]");
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("option inconnue : {other}");
                return ExitCode::from(2);
            }
        }
    }
    match fakecam::spawn_on(&listen, cfg) {
        Ok(h) => {
            println!("fakecam écoute sur {} (Ctrl+C pour arrêter)", h.addr);
            loop {
                std::thread::park();
            }
        }
        Err(e) => {
            eprintln!("impossible d'écouter sur {listen} : {e}");
            ExitCode::FAILURE
        }
    }
}
