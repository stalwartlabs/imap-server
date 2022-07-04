use std::borrow::Cow;

use crate::{core::receiver::Token, protocol::copy};

use super::parse_sequence_set;

pub fn parse_copy(tokens: Vec<Token>) -> super::Result<copy::Arguments> {
    if tokens.len() > 1 {
        let mut tokens = tokens.into_iter();

        Ok(copy::Arguments {
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
        protocol::{copy, Sequence},
    };

    #[test]
    fn parse_copy() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [(
            "A003 COPY 2:4 MEETING\r\n",
            copy::Arguments {
                sequence_set: vec![Sequence::Range {
                    start: 2.into(),
                    end: 4.into(),
                }],
                mailbox_name: "MEETING".to_string(),
            },
        )] {
            assert_eq!(
                super::parse_copy(
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
