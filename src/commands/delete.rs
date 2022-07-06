use crate::{
    core::{
        client::{Session, SessionData},
        receiver::Request,
    },
    protocol::delete::Arguments,
};
use std::sync::Arc;

impl Session {
    pub async fn handle_delete(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_delete(self.version) {
            Ok(arguments) => {
                spawn_delete(self.state.session_data(), arguments);
                Ok(())
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}

fn spawn_delete(data: Arc<SessionData>, arguments: Arguments) {
    tokio::spawn(async move {});
}
