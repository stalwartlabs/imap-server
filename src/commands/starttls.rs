use tokio::{io::WriteHalf, net::TcpStream, sync::oneshot};
use tracing::debug;

use crate::core::{client::Session, receiver::Request, writer, StatusResponse};

impl Session {
    pub async fn handle_starttls(
        &mut self,
        request: Request,
    ) -> Result<Option<WriteHalf<TcpStream>>, ()> {
        self.write_bytes(
            StatusResponse::ok(request.tag.into(), None, "Begin TLS negotiation now").into_bytes(),
        )
        .await?;

        // Recover WriteHalf from writer
        let (tx, rx) = oneshot::channel();
        if let Err(err) = self.writer.send(writer::Event::Upgrade(tx)).await {
            debug!("Failed to write to channel: {}", err);
            return Err(());
        }
        if let Ok(event) = rx.await {
            if let writer::Event::Stream(stream_tx) = event {
                Ok(Some(stream_tx))
            } else {
                unreachable!()
            }
        } else {
            debug!("Failed to read from channel");
            Err(())
        }
    }
}
