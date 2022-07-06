use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
    },
    protocol::create::Arguments,
};
use std::sync::Arc;

impl Session {
    pub async fn handle_create(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_create(self.version) {
            Ok(arguments) => {
                spawn_create(self.state.session_data(), arguments);
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

fn spawn_create(data: Arc<SessionData>, arguments: Arguments) {
    tokio::spawn(async move {});
}
