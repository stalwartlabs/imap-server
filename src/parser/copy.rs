use crate::{core::receiver::Request, protocol::copy};

use super::parse_sequence_set;

pub fn parse_copy(request: Request) -> crate::core::Result<copy::Arguments> {
    if request.tokens.len() > 1 {
        let mut tokens = request.tokens.into_iter();

        Ok(copy::Arguments {
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
        protocol::{copy, Sequence},
    };

    #[test]
    fn parse_copy() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [(
            "A003 COPY 2:4 MEETING\r\n",
            copy::Arguments {
                sequence_set: Sequence::Range {
                    start: 2.into(),
                    end: 4.into(),
                },
                mailbox_name: "MEETING".to_string(),
            },
        )] {
            assert_eq!(
                super::parse_copy(receiver.parse(&mut command.as_bytes().iter()).unwrap()).unwrap(),
                arguments
            );
        }
    }
}
