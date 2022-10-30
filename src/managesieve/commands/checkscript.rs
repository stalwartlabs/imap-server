/*
 * Copyright (c) 2020-2022, Stalwart Labs Ltd.
 *
 * This file is part of the Stalwart IMAP Server.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of
 * the License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 * in the LICENSE file at the top-level directory of this distribution.
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * You can be released from the requirements of the AGPLv3 license by
 * purchasing a commercial license. Please contact licensing@stalw.art
 * for more details.
*/

use crate::{
    core::receiver::Request,
    managesieve::{client::Session, Command, StatusResponse},
};

use super::IntoStatusResponse;

impl Session {
    pub async fn handle_checkscript(
        &mut self,
        request: Request<Command>,
    ) -> Result<bool, StatusResponse> {
        if request.tokens.is_empty() {
            return Err(StatusResponse::no("Expected script as a parameter."));
        }

        self.client()
            .sieve_script_validate(request.tokens.into_iter().next().unwrap().unwrap_bytes())
            .await
            .map_err(|err| err.into_status_response())?;

        Ok(self
            .write_bytes(StatusResponse::ok("Script is valid.").into_bytes())
            .await
            .is_ok())
    }
}
