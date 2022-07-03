pub mod core;
pub mod parser;
pub mod protocol;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    Ok(())
}
