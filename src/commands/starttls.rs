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

use tokio::{io::WriteHalf, net::TcpStream, sync::oneshot};
use tracing::debug;

use crate::core::{client::Session, receiver::Request, writer, StatusResponse};

impl Session {
    pub async fn handle_starttls(
        &mut self,
        request: Request,
    ) -> Result<Option<WriteHalf<TcpStream>>, ()> {
        self.write_bytes(
            StatusResponse::ok("Begin TLS negotiation now")
                .with_tag(request.tag)
                .into_bytes(),
        )
        .await?;

        // Recover WriteHalf from writer
        let (tx, rx) = oneshot::channel();
        if let Err(err) = self.writer.send(writer::Event::Upgrade(tx)).await {
            debug!("Failed to write to channel: {}", err);
            return Err(());
        }
        if let Ok(event) = rx.await {
            if let writer::Event::Stream(stream_tx) = event {
                Ok(Some(stream_tx))
            } else {
                unreachable!()
            }
        } else {
            debug!("Failed to read from channel");
            Err(())
        }
    }
}
