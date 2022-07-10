use crate::{core::receiver::Request, protocol::move_};

use super::parse_sequence_set;

pub fn parse_move(request: Request) -> crate::core::Result<move_::Arguments> {
    if request.tokens.len() > 1 {
        let mut tokens = request.tokens.into_iter();

        Ok(move_::Arguments {
            sequence_set: parse_sequence_set(
                &tokens
                    .next()
                    .ok_or((request.tag.as_str(), "Missing sequence set."))?
                    .unwrap_bytes(),
            )
            .map_err(|v| (request.tag.as_str(), v))?,
            mailbox_name: tokens
                .next()
                .ok_or((request.tag.as_str(), "Missing mailbox name."))?
                .unwrap_string()
                .map_err(|v| (request.tag.as_str(), v))?,
        })
    } else {
        Err(request.into_error("Missing arguments."))
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
                sequence_set: Sequence::Range {
                    start: 42.into(),
                    end: 69.into(),
                },
                mailbox_name: "foo".to_string(),
            },
        )] {
            assert_eq!(
                super::parse_move(receiver.parse(&mut command.as_bytes().iter()).unwrap()).unwrap(),
                arguments
            );
        }
    }
}
