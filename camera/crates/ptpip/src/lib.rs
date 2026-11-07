//! Client PTP/IP minimal pour appareils photo Wi-Fi.
//!
//! * PTP : ISO 15740 (jeux de données, opérations, propriétés, événements).
//! * PTP/IP : CIPA DC-X005 (deux connexions TCP, port 15740).
//! * Extensions Nikon utiles aux COOLPIX Wi-Fi (visée, déclenchement, événements).
//!
//! Aucune opération n'est envoyée sans appel explicite : se connecter et
//! lire `GetDeviceInfo` ne modifie rien sur l'appareil.

pub mod client;
pub mod codes;
pub mod dataset;
pub mod error;
pub mod net;
pub mod packet;

pub use client::{extract_jpeg, Camera, DataSink, Event, Options, Response};
pub use error::{Error, Result};
