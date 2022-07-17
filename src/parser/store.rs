use std::borrow::Cow;

use crate::{
    core::{
        receiver::{Request, Token},
        Flag,
    },
    protocol::store::{self, Operation},
};

use super::{parse_long_integer, parse_sequence_set};

impl Request {
    pub fn parse_store(self) -> crate::core::Result<store::Arguments> {
        let mut tokens = self.tokens.into_iter().peekable();

        // Sequence set
        let sequence_set = parse_sequence_set(
            &tokens
                .next()
                .ok_or((self.tag.as_str(), "Missing sequence set."))?
                .unwrap_bytes(),
        )
        .map_err(|v| (self.tag.as_str(), v))?;
        let mut unchanged_since = None;

        // CONDSTORE parameters
        if let Some(Token::ParenthesisOpen) = tokens.peek() {
            tokens.next();
            while let Some(token) = tokens.next() {
                match token {
                    Token::Argument(param) if param.eq_ignore_ascii_case(b"UNCHANGEDSINCE") => {
                        unchanged_since = parse_long_integer(
                            &tokens
                                .next()
                                .ok_or((self.tag.as_str(), "Missing UNCHANGEDSINCE parameter."))?
                                .unwrap_bytes(),
                        )
                        .map_err(|v| (self.tag.as_str(), v))?
                        .into();
                    }
                    Token::ParenthesisClose => {
                        break;
                    }
                    _ => {
                        return Err((
                            self.tag.as_str(),
                            Cow::from(format!("Unsupported parameter '{}'.", token)),
                        )
                            .into());
                    }
                }
            }
        }

        // Operation
        let operation = tokens
            .next()
            .ok_or((self.tag.as_str(), "Missing message data item name."))?
            .unwrap_bytes();
        let (is_silent, operation) = if operation.eq_ignore_ascii_case(b"FLAGS") {
            (false, Operation::Set)
        } else if operation.eq_ignore_ascii_case(b"FLAGS.SILENT") {
            (true, Operation::Set)
        } else if operation.eq_ignore_ascii_case(b"+FLAGS") {
            (false, Operation::Add)
        } else if operation.eq_ignore_ascii_case(b"+FLAGS.SILENT") {
            (true, Operation::Add)
        } else if operation.eq_ignore_ascii_case(b"-FLAGS") {
            (false, Operation::Clear)
        } else if operation.eq_ignore_ascii_case(b"-FLAGS.SILENT") {
            (true, Operation::Clear)
        } else {
            return Err((
                self.tag,
                format!(
                    "Unsupported message data item name: {:?}",
                    String::from_utf8_lossy(&operation)
                ),
            )
                .into());
        };

        // Flags
        if tokens
            .next()
            .map_or(true, |token| !token.is_parenthesis_open())
        {
            return Err((self.tag, "Expected store parameters between parentheses.").into());
        }

        let mut keywords = Vec::new();
        for token in tokens {
            match token {
                Token::Argument(flag) => {
                    keywords.push(Flag::parse_imap(flag).map_err(|v| (self.tag.as_str(), v))?);
                }
                Token::ParenthesisClose => {
                    break;
                }
                _ => {
                    return Err((self.tag.as_str(), "Unsupported flag.").into());
                }
            }
        }

        if !keywords.is_empty() {
            Ok(store::Arguments {
                tag: self.tag,
                sequence_set,
                operation,
                is_silent,
                keywords,
                unchanged_since,
            })
        } else {
            Err((self.tag.as_str(), "Missing flags.").into())
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        core::{receiver::Receiver, Flag},
        protocol::{
            store::{self, Operation},
            Sequence,
        },
    };

    #[test]
    fn parse_store() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A003 STORE 2:4 +FLAGS (\\Deleted)\r\n",
                store::Arguments {
                    sequence_set: Sequence::Range {
                        start: 2.into(),
                        end: 4.into(),
                    },
                    is_silent: false,
                    operation: Operation::Add,
                    keywords: vec![Flag::Deleted],
                    tag: "A003".to_string(),
                    unchanged_since: None,
                },
            ),
            (
                "A004 STORE *:100 -FLAGS.SILENT ($Phishing $Junk)\"\r\n",
                store::Arguments {
                    sequence_set: Sequence::Range {
                        start: None,
                        end: 100.into(),
                    },
                    is_silent: true,
                    operation: Operation::Clear,
                    keywords: vec![Flag::Phishing, Flag::Junk],
                    tag: "A004".to_string(),
                    unchanged_since: None,
                },
            ),
            (
                "d105 STORE 7,5,9 (UNCHANGEDSINCE 320162338) +FLAGS.SILENT (\\Deleted)\"\r\n",
                store::Arguments {
                    sequence_set: Sequence::List {
                        items: vec![
                            Sequence::Number { value: 7 },
                            Sequence::Number { value: 5 },
                            Sequence::Number { value: 9 },
                        ],
                    },
                    is_silent: true,
                    operation: Operation::Add,
                    keywords: vec![Flag::Deleted],
                    tag: "d105".to_string(),
                    unchanged_since: Some(320162338),
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_store()
                    .unwrap(),
                arguments
            );
        }
    }
}
