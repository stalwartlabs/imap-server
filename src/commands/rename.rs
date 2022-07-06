use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
    },
    protocol::rename::Arguments,
};
use std::sync::Arc;

impl Session {
    pub async fn handle_rename(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_rename(self.version) {
            Ok(arguments) => {
                spawn_rename(self.state.session_data(), arguments);
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

fn spawn_rename(data: Arc<SessionData>, arguments: Arguments) {
    tokio::spawn(async move {});
}
