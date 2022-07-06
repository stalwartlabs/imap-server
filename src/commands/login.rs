use jmap_client::client::Credentials;

use crate::core::{client::Session, receiver::Request};

impl Session {
    pub async fn handle_login(&mut self, request: Request) -> Result<(), ()> {
        match request.parse_login() {
            Ok(args) => {
                self.authenticate(Credentials::basic(&args.username, &args.password), args.tag)
                    .await
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}
