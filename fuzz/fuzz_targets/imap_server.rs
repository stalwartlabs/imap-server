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
