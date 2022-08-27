use std::{fs::File, io::BufReader, sync::Arc};

use rustls::{Certificate, PrivateKey};
use rustls_pemfile::{certs, pkcs8_private_keys};
use tracing::warn;

use super::{env_settings::EnvSettings, Core};

pub const DEFAULT_JMAP_URL: &str = "http://127.0.0.1:8080";

pub fn build_core(settings: &EnvSettings) -> Core {
    Core {
        db: Arc::new(
            sled::open(
                settings
                    .get("cache-dir")
                    .failed_to("start server: Missing cache-dir parameter."),
            )
            .failed_to("open database"),
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
        folder_all: if let Some(folder_all) = settings.get("name-all") {
            folder_all
        } else {
            "All Mail".to_string()
        },
        max_request_size: settings
            .parse("max-request-size")
            .unwrap_or(50 * 1024 * 1024),
        trusted_hosts: if let Some(folder_shared) = settings.get("jmap-trusted-hosts") {
            folder_shared
                .split(';')
                .into_iter()
                .map(|host| host.to_string())
                .collect()
        } else {
            vec!["127.0.0.1".to_string()]
        },
    }
}

pub fn load_tls_config(settings: &EnvSettings) -> rustls::ServerConfig {
    let (cert_path, key_path) = if let (Some(cert_path), Some(key_path)) =
        (settings.get("cert-path"), settings.get("key-path"))
    {
        (cert_path, key_path)
    } else {
        failed_to("load TLS config: Missing 'cert-path' and/or 'key-path' parameters.");
    };

    let certificates: Vec<Certificate> = certs(&mut BufReader::new(
        File::open(&cert_path).failed_to("open certificate path"),
    ))
    .failed_to("load TLS config: Invalid certificate file")
    .into_iter()
    .map(Certificate)
    .collect();

    let mut private_keys: Vec<PrivateKey> = pkcs8_private_keys(&mut BufReader::new(
        File::open(&key_path).failed_to("open private key path"),
    ))
    .failed_to("load TLS config: Invalid private key file")
    .into_iter()
    .map(PrivateKey)
    .collect();

    if certificates.is_empty() {
        failed_to(&format!(
            "load TLS config: No certificates found in file {}",
            &cert_path
        ));
    }

    if private_keys.is_empty() {
        failed_to(&format!(
            "load TLS config: No private keys found in file {}",
            &key_path
        ));
    }

    rustls::ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(certificates, private_keys.remove(0))
        .failed_to("load TLS configuration")
}

pub trait UnwrapFailure<T> {
    fn failed_to(self, action: &str) -> T;
}

impl<T> UnwrapFailure<T> for Option<T> {
    fn failed_to(self, message: &str) -> T {
        match self {
            Some(result) => result,
            None => {
                println!("Failed to {}", message);
                std::process::exit(1);
            }
        }
    }
}

impl<T, E: std::fmt::Display> UnwrapFailure<T> for Result<T, E> {
    fn failed_to(self, message: &str) -> T {
        match self {
            Ok(result) => result,
            Err(err) => {
                println!("Failed to {}: {}", message, err);
                std::process::exit(1);
            }
        }
    }
}

pub fn failed_to(action: &str) -> ! {
    println!("Failed to {}", action);
    std::process::exit(1);
}
