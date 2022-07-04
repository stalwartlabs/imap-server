use std::borrow::Cow;

use crate::{core::receiver::Token, protocol::move_};

use super::parse_sequence_set;

pub fn parse_move(tokens: Vec<Token>) -> super::Result<move_::Arguments> {
    if tokens.len() > 1 {
        let mut tokens = tokens.into_iter();

        Ok(move_::Arguments {
            sequence_set: parse_sequence_set(
                &tokens
                    .next()
                    .ok_or_else(|| Cow::from("Missing sequence set."))?
                    .unwrap_bytes(),
            )?,
            mailbox_name: tokens
                .next()
                .ok_or_else(|| Cow::from("Missing mailbox name."))?
                .unwrap_string()?,
        })
    } else {
        Err("Missing arguments.".into())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::receiver::Receiver,
        protocol::{move_, Sequence},
    };

    #[test]
    fn parse_move() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [(
            "a UID MOVE 42:69 foo\r\n",
            move_::Arguments {
                sequence_set: vec![Sequence::Range {
                    start: 42.into(),
                    end: 69.into(),
                }],
                mailbox_name: "foo".to_string(),
            },
        )] {
            assert_eq!(
                super::parse_move(
                    receiver
                        .parse(&mut command.as_bytes().iter())
                        .unwrap()
                        .tokens
                )
                .unwrap(),
                arguments
            );
        }
    }
}
