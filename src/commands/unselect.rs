use crate::core::{
    client::{Session, State},
    receiver::Request,
    Command, StatusResponse,
};

impl Session {
    pub async fn handle_unselect(&mut self, request: Request) -> Result<(), ()> {
        self.state = State::Authenticated {
            data: self.state.session_data(),
        };
        self.write_bytes(
            StatusResponse::completed(Command::Unselect)
                .with_tag(request.tag)
                .into_bytes(),
        )
        .await
    }
}
