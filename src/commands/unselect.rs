use crate::core::{
    client::{Session, State},
    receiver::Request,
    StatusResponse,
};

impl Session {
    pub async fn handle_unselect(&mut self, request: Request) -> Result<(), ()> {
        self.state = State::Authenticated {
            data: self.state.session_data(),
        };
        self.write_bytes(
            StatusResponse::ok(request.tag.into(), None, "UNSELECT completed").into_bytes(),
        )
        .await
    }
}
