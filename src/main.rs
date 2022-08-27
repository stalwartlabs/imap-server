use imap_server::core::env_settings::EnvSettings;
use imap_server::start_imap_server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Start server
    start_imap_server(EnvSettings::new()).await
}
