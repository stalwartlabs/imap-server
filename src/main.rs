use stalwart_imap::core::env_settings::EnvSettings;
use stalwart_imap::start_imap_server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // Start server
    start_imap_server(EnvSettings::new()).await
}
