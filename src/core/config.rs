use std::{fs::File, io::BufReader, sync::Arc};

use rustls::{Certificate, PrivateKey};
use rustls_pemfile::{certs, pkcs8_private_keys};
use tracing::warn;

use super::{env_settings::EnvSettings, Core};

pub const DEFAULT_JMAP_URL: &str = "http://127.0.0.1/.well-known/jmap";

pub fn load_config(settings: &EnvSettings) -> Core {
    Core {
        db: Arc::new(
            sled::open(
                settings
                    .get("cache-dir")
                    .expect("Missing cache-dir parameter."),
            )
            .expect("Failed to open database"),
        ),
        worker_pool: rayon::ThreadPoolBuilder::new()
            .num_threads(
                settings
                    .parse("worker-pool-size")
                    .filter(|v| *v > 0)
                    .unwrap_or_else(num_cpus::get),
            )
            .build()
            .unwrap(),
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
