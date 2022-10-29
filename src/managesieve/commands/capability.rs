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

use crate::managesieve::{
    client::{Session, State},
    StatusResponse,
};

const EXTENSIONS_ALL: &[u8] = b"\"SIEVE\" \"body comparator-elbonia comparator-i;ascii-casemap comparator-i;ascii-numeric comparator-i;octet convert copy date duplicate editheader enclose encoded-character enotify envelope envelope-deliverby envelope-dsn environment ereject extlists extracttext fcc fileinto foreverypart ihave imap4flags imapsieve include index mailbox mailboxid mboxmetadata mime redirect-deliverby redirect-dsn regex reject relational replace servermetadata spamtest spamtestplus special-use subaddress vacation vacation-seconds variables virustest\"\r\n";

impl Session {
    pub async fn handle_capability(
        &mut self,
        message: &'static str,
    ) -> Result<bool, StatusResponse> {
        let mut response = Vec::with_capacity(128);
        response.extend_from_slice(b"\"IMPLEMENTATION\" \"Stalwart ManageSieve v");
        response.extend_from_slice(env!("CARGO_PKG_VERSION").as_bytes());
        response.extend_from_slice(b"\"\r\n");
        response.extend_from_slice(b"\"VERSION\" \"1.0\"\r\n");
        if !self.is_tls {
            response.extend_from_slice(b"\"SASL\" \"\"\r\n");
            response.extend_from_slice(b"\"STARTTLS\"\r\n");
        } else {
            response.extend_from_slice(b"\"SASL\" \"PLAIN OAUTHBEARER\"\r\n");
        };
        if let State::Authenticated { client, .. } = &self.state {
            let session = client.session();
            let sieve = session.sieve_capabilities().unwrap();
            response.extend_from_slice(b"\"SIEVE\" \"");
            response.extend_from_slice(sieve.sieve_extensions().join(" ").as_bytes());
            response.extend_from_slice(b"\"\r\n");
            if let Some(notification_methods) = sieve.notification_methods() {
                response.extend_from_slice(b"\"NOTIFY\" \"");
                response.extend_from_slice(notification_methods.join(" ").as_bytes());
                response.extend_from_slice(b"\"\r\n");
            }
            if let Some(max_redirects) = sieve.max_number_redirects() {
                response.extend_from_slice(b"\"MAXREDIRECTS\" \"");
                response.extend_from_slice(max_redirects.to_string().as_bytes());
                response.extend_from_slice(b"\"\r\n");
            }
        } else {
            response.extend_from_slice(EXTENSIONS_ALL);
        }

        Ok(self
            .write_bytes(StatusResponse::ok(message).serialize(response))
            .await
            .is_ok())
    }
}
