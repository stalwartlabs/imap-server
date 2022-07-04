use std::borrow::Cow;

use crate::{core::receiver::Token, protocol::lsub};

pub fn parse_lsub(tokens: Vec<Token>) -> super::Result<lsub::Arguments> {
    if tokens.len() > 1 {
        let mut tokens = tokens.into_iter();

        Ok(lsub::Arguments {
            reference_name: tokens
                .next()
                .ok_or_else(|| Cow::from("Missing reference name."))?
                .unwrap_string()?,
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
    use crate::{core::receiver::Receiver, protocol::lsub};

    #[test]
    fn parse_lsub() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A002 LSUB \"#news.\" \"comp.mail.*\"\r\n",
                lsub::Arguments {
                    reference_name: "#news.".to_string(),
                    mailbox_name: "comp.mail.*".to_string(),
                },
            ),
            (
                "A002 LSUB \"#news.\" \"comp.%\"\r\n",
                lsub::Arguments {
                    reference_name: "#news.".to_string(),
                    mailbox_name: "comp.%".to_string(),
                },
            ),
        ] {
            assert_eq!(
                super::parse_lsub(
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
