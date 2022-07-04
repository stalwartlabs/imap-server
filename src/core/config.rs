use std::{fs::File, io::BufReader, sync::Arc};

use rustls::{Certificate, PrivateKey};
use rustls_pemfile::{certs, pkcs8_private_keys, rsa_private_keys};
use tracing::warn;

use super::env_settings::EnvSettings;

pub struct Config {
    pub tls_acceptor: tokio_rustls::TlsAcceptor,
    pub jmap_url: String,
    pub folder_shared: String,
    pub folder_all: String,
}

pub const DEFAULT_JMAP_URL: &str = "https://127.0.0.1";

pub fn load_config(settings: &EnvSettings) -> Config {
    Config {
        tls_acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(load_tls_config(settings))),
        jmap_url: if let Some(jmap_url) = settings.get("jmap-url") {
            jmap_url
        } else {
            warn!("No jmap-url specified, using default: {}", DEFAULT_JMAP_URL);
            DEFAULT_JMAP_URL.to_string()
        },
        folder_shared: if let Some(folder_shared) = settings.get("name-shared") {
            folder_shared
        } else {
            "Shared Folders".to_string()
        },
        folder_all: if let Some(folder_shared) = settings.get("name-shared") {
            folder_shared
        } else {
            "All Messages".to_string()
        },
    }
}

pub fn load_tls_config(settings: &EnvSettings) -> rustls::ServerConfig {
    let (cert_path, key_path) = if let (Some(cert_path), Some(key_path)) =
        (settings.get("cert-path"), settings.get("key-path"))
    {
        (cert_path, key_path)
    } else {
        panic!("Missing TLS 'cert-path' and/or 'key-path' parameters.");
    };

    let certificates: Vec<Certificate> = certs(&mut BufReader::new(
        File::open(&cert_path).expect("Failed to open certificate path"),
    ))
    .expect("Invalid certificate file")
    .into_iter()
    .map(Certificate)
    .collect();

    let mut private_keys: Vec<PrivateKey> = pkcs8_private_keys(&mut BufReader::new(
        File::open(&key_path).expect("Failed to open private key path"),
    ))
    .expect("Invalid private key file")
    .into_iter()
    .map(PrivateKey)
    .collect();

    if certificates.is_empty() {
        panic!("No certificates found in file {}", &cert_path);
    }

    if private_keys.is_empty() {
        panic!("No private keys found in file {}", &key_path);
    }

    rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certificates, private_keys.remove(0))
        .expect("Failed to load TLS configuration")
}
