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

impl Request {
    #[allow(clippy::while_let_on_iterator)]
    pub fn parse_thread(self) -> crate::core::Result<thread::Arguments> {
        if self.tokens.is_empty() {
            return Err(self.into_error("Missing thread criteria."));
        }

        let mut tokens = self.tokens.into_iter().peekable();
        let algorithm = Algorithm::parse(
            &tokens
                .next()
                .ok_or((self.tag.as_str(), "Missing threading algorithm."))?
                .unwrap_bytes(),
        )
        .map_err(|v| (self.tag.as_str(), v))?;

        let decoder = get_charset_decoder(
            &tokens
                .next()
                .ok_or((self.tag.as_str(), "Missing charset."))?
                .unwrap_bytes(),
        );

        let mut filters =
            parse_filters(&mut tokens, decoder).map_err(|v| (self.tag.as_str(), v))?;
        match filters.len() {
            0 => Err((self.tag.as_str(), "No filters found in command.").into()),
            1 => Ok(thread::Arguments {
                algorithm,
                filter: filters.pop().unwrap(),
                tag: self.tag,
            }),
            _ => Ok(thread::Arguments {
                algorithm,
                filter: Filter::Operator(Operator::And, filters),
                tag: self.tag,
            }),
        }
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
                    tag: "A283".to_string(),
                },
            ),
            (
                b"A284 THREAD REFERENCES US-ASCII TEXT \"gewp\"\r\n".to_vec(),
                thread::Arguments {
                    algorithm: Algorithm::References,
                    filter: Filter::Text("gewp".to_string()),
                    tag: "A284".to_string(),
                },
            ),
        ] {
            let command_str = String::from_utf8_lossy(&command).into_owned();

            assert_eq!(
                receiver
                    .parse(&mut command.iter())
                    .unwrap()
                    .parse_thread()
                    .expect(&command_str),
                arguments,
                "{}",
                command_str
            );
        }
    }
}
