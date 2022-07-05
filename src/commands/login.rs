use jmap_client::client::Credentials;

use crate::{
    core::{client::Session, receiver::Request},
    parser::login::parse_login,
};

impl Session {
    pub async fn handle_login(&mut self, request: Request) -> Result<(), ()> {
        match parse_login(request) {
            Ok(args) => {
                self.authenticate(Credentials::basic(&args.username, &args.password), args.tag)
                    .await
            }
            Err(response) => self.write_bytes(response.into_bytes()).await,
        }
    }
}
