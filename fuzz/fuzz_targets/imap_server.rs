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

#![no_main]
use imap_server::core::receiver::{Receiver, State};
use libfuzzer_sys::fuzz_target;

static IMAP_ALPHABET: &[u8] = b"()[]<>{}+-.:=\"NIL012345ABCDEF ";

fuzz_target!(|data: &[u8]| {
    let imap_data = data
        .iter()
        .map(|&byte| IMAP_ALPHABET[byte as usize % IMAP_ALPHABET.len()])
        .collect::<Vec<_>>();

    for state in [
        State::Start,
        State::Tag,
        State::Command { is_uid: false },
        State::Argument { last_ch: 0 },
        State::Argument {
            last_ch: data.get(0).copied().unwrap_or(u8::MAX),
        },
        State::ArgumentQuoted { escaped: true },
        State::ArgumentQuoted { escaped: false },
        State::Literal { non_sync: true },
        State::Literal { non_sync: false },
        State::LiteralData { remaining: 8192 },
    ] {
        let mut r = Receiver::new();
        r.state = state;
        r.parse(&mut data.iter()).ok();

        let mut r = Receiver::new();
        r.state = state;
        r.parse(&mut imap_data.iter()).ok();
    }
});
