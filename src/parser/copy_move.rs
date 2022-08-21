use crate::{core::receiver::Request, protocol::copy_move};

use super::parse_sequence_set;

impl Request {
    pub fn parse_copy_move(self) -> crate::core::Result<copy_move::Arguments> {
        if self.tokens.len() > 1 {
            let mut tokens = self.tokens.into_iter();

            Ok(copy_move::Arguments {
                sequence_set: parse_sequence_set(
                    &tokens
                        .next()
                        .ok_or((self.tag.as_str(), "Missing sequence set."))?
                        .unwrap_bytes(),
                )
                .map_err(|v| (self.tag.as_str(), v))?,
                mailbox_name: tokens
                    .next()
                    .ok_or((self.tag.as_str(), "Missing mailbox name."))?
                    .unwrap_string()
                    .map_err(|v| (self.tag.as_str(), v))?,
                tag: self.tag,
            })
        } else {
            Err(self.into_error("Missing arguments."))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        core::receiver::Receiver,
        protocol::{copy_move, Sequence},
    };

    #[test]
    fn parse_copy() {
        let mut receiver = Receiver::new();

        assert_eq!(
            receiver
                .parse(&mut "A003 COPY 2:4 MEETING\r\n".as_bytes().iter())
                .unwrap()
                .parse_copy_move()
                .unwrap(),
            copy_move::Arguments {
                sequence_set: Sequence::Range {
                    start: 2.into(),
                    end: 4.into(),
                },
                mailbox_name: "MEETING".to_string(),
                tag: "A003".to_string(),
            }
        );
    }
}
