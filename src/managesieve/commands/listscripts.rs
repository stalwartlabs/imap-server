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

use jmap_client::sieve::Property;

use crate::managesieve::{client::Session, StatusResponse};

use super::IntoStatusResponse;

impl Session {
    pub async fn handle_listscripts(&mut self) -> Result<bool, StatusResponse> {
        let mut request = self.client().build();
        request
            .get_sieve_script()
            .properties([Property::Name, Property::IsActive]);

        let mut response = Vec::with_capacity(128);

        for script in request
            .send_get_sieve_script()
            .await
            .map_err(|err| err.into_status_response())?
            .take_list()
        {
            response.push(b'\"');
            if let Some(name) = script.name() {
                for ch in name.as_bytes() {
                    if [b'\\', b'\"'].contains(ch) {
                        response.push(b'\\');
                    }
                    response.push(*ch);
                }
            }

            if script.is_active() {
                response.extend_from_slice(b"\" ACTIVE\r\n");
            } else {
                response.extend_from_slice(b"\"\r\n");
            }
        }

        Ok(self
            .write_bytes(StatusResponse::ok("").serialize(response))
            .await
            .is_ok())
    }
}
