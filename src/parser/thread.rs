use jmap_client::core::query::Operator;
use mail_parser::decoders::charsets::map::get_charset_decoder;

use crate::{
    core::receiver::Request,
    protocol::{
        search::Filter,
        thread::{self, Algorithm},
    },
};

use super::search::parse_filters;

#[allow(clippy::while_let_on_iterator)]
pub fn parse_thread(request: Request) -> crate::core::Result<thread::Arguments> {
    if request.tokens.is_empty() {
        return Err(request.into_error("Missing thread criteria."));
    }

    let mut tokens = request.tokens.into_iter().peekable();
    let algorithm = Algorithm::parse(
        &tokens
            .next()
            .ok_or((request.tag.as_str(), "Missing threading algorithm."))?
            .unwrap_bytes(),
    )
    .map_err(|v| (request.tag.as_str(), v))?;

    let decoder = get_charset_decoder(
        &tokens
            .next()
            .ok_or((request.tag.as_str(), "Missing charset."))?
            .unwrap_bytes(),
    );

    let mut filters = parse_filters(&mut tokens, decoder).map_err(|v| (request.tag.as_str(), v))?;
    match filters.len() {
        0 => Err((request.tag.as_str(), "No filters found in command.").into()),
        1 => Ok(thread::Arguments {
            algorithm,
            filter: filters.pop().unwrap(),
        }),
        _ => Ok(thread::Arguments {
            algorithm,
            filter: Filter::Operator(Operator::And, filters),
        }),
    }
}

impl Algorithm {
    pub fn parse(value: &[u8]) -> super::Result<Self> {
        if value.eq_ignore_ascii_case(b"ORDEREDSUBJECT") {
            Ok(Self::OrderedSubject)
        } else if value.eq_ignore_ascii_case(b"REFERENCES") {
            Ok(Self::References)
        } else {
            Err(format!(
                "Invalid threading algorithm {:?}",
                String::from_utf8_lossy(value)
            )
            .into())
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        core::receiver::Receiver,
        protocol::{
            search::Filter,
            thread::{self, Algorithm},
        },
    };

    #[test]
    fn parse_thread() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                b"A283 THREAD ORDEREDSUBJECT UTF-8 SINCE 5-MAR-2000\r\n".to_vec(),
                thread::Arguments {
                    algorithm: Algorithm::OrderedSubject,
                    filter: Filter::Since(952214400),
                },
            ),
            (
                b"A284 THREAD REFERENCES US-ASCII TEXT \"gewp\"\r\n".to_vec(),
                thread::Arguments {
                    algorithm: Algorithm::References,
                    filter: Filter::Text("gewp".to_string()),
                },
            ),
        ] {
            let command_str = String::from_utf8_lossy(&command).into_owned();

            assert_eq!(
                super::parse_thread(receiver.parse(&mut command.iter()).unwrap())
                    .expect(&command_str),
                arguments,
                "{}",
                command_str
            );
        }
    }
}
