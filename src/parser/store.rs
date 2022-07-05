use crate::{
    core::{
        receiver::{Request, Token},
        Flag,
    },
    protocol::store::{self, Operation},
};

use super::parse_sequence_set;

pub fn parse_store(request: Request) -> crate::core::Result<store::Arguments> {
    let mut tokens = request.tokens.into_iter();
    let sequence_set = parse_sequence_set(
        &tokens
            .next()
            .ok_or((request.tag.as_str(), "Missing sequence set."))?
            .unwrap_bytes(),
    )
    .map_err(|v| (request.tag.as_str(), v))?;
    let operation = tokens
        .next()
        .ok_or((request.tag.as_str(), "Missing message data item name."))?
        .unwrap_bytes();
    let operation = if operation.eq_ignore_ascii_case(b"FLAGS") {
        Operation::Set
    } else if operation.eq_ignore_ascii_case(b"FLAGS.SILENT") {
        Operation::SetSilent
    } else if operation.eq_ignore_ascii_case(b"+FLAGS") {
        Operation::Add
    } else if operation.eq_ignore_ascii_case(b"+FLAGS.SILENT") {
        Operation::AddSilent
    } else if operation.eq_ignore_ascii_case(b"-FLAGS") {
        Operation::Clear
    } else if operation.eq_ignore_ascii_case(b"-FLAGS.SILENT") {
        Operation::ClearSilent
    } else {
        return Err((
            request.tag,
            format!(
                "Unsupported message data item name: {:?}",
                String::from_utf8_lossy(&operation)
            ),
        )
            .into());
    };

    if tokens
        .next()
        .map_or(true, |token| !token.is_parenthesis_open())
    {
        return Err((
            request.tag,
            "Expected store parameters between parentheses.",
        )
            .into());
    }

    let mut keywords = Vec::new();
    for token in tokens {
        match token {
            Token::Argument(flag) => {
                keywords.push(Flag::parse_imap(flag).map_err(|v| (request.tag.as_str(), v))?);
            }
            Token::ParenthesisClose => {
                break;
            }
            _ => {
                return Err((request.tag.as_str(), "Unsupported flag.").into());
            }
        }
    }

    if !keywords.is_empty() {
        Ok(store::Arguments {
            sequence_set,
            operation,
            keywords,
        })
    } else {
        Err((request.tag.as_str(), "Missing flags.").into())
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
                    sequence_set: vec![Sequence::Range {
                        start: 2.into(),
                        end: 4.into(),
                    }],
                    operation: Operation::Add,
                    keywords: vec![Flag::Deleted],
                },
            ),
            (
                "A004 STORE *:100 -FLAGS.SILENT ($Phishing $Junk)\"\r\n",
                store::Arguments {
                    sequence_set: vec![Sequence::Range {
                        start: None,
                        end: 100.into(),
                    }],
                    operation: Operation::ClearSilent,
                    keywords: vec![Flag::Phishing, Flag::Junk],
                },
            ),
        ] {
            assert_eq!(
                super::parse_store(receiver.parse(&mut command.as_bytes().iter()).unwrap())
                    .unwrap(),
                arguments
            );
        }
    }
}
