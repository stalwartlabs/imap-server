use crate::{
    core::{receiver::Request, utf7::utf7_maybe_decode},
    protocol::{
        list::{self, SelectionOption},
        ProtocolVersion,
    },
};

impl Request {
    pub fn parse_lsub(self) -> crate::core::Result<list::Arguments> {
        if self.tokens.len() > 1 {
            let mut tokens = self.tokens.into_iter();

            Ok(list::Arguments::Extended {
                reference_name: tokens
                    .next()
                    .ok_or((self.tag.as_str(), "Missing reference name."))?
                    .unwrap_string()
                    .map_err(|v| (self.tag.as_str(), v))?,
                mailbox_name: vec![utf7_maybe_decode(
                    tokens
                        .next()
                        .ok_or((self.tag.as_str(), "Missing mailbox name."))?
                        .unwrap_string()
                        .map_err(|v| (self.tag.as_str(), v))?,
                    ProtocolVersion::Rev1,
                )],
                selection_options: vec![SelectionOption::Subscribed],
                return_options: vec![],
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
        protocol::list::{self, SelectionOption},
    };

    #[test]
    fn parse_lsub() {
        let mut receiver = Receiver::new();

        for (command, arguments) in [
            (
                "A002 LSUB \"#news.\" \"comp.mail.*\"\r\n",
                list::Arguments::Extended {
                    tag: "A002".to_string(),
                    reference_name: "#news.".to_string(),
                    mailbox_name: vec!["comp.mail.*".to_string()],
                    selection_options: vec![SelectionOption::Subscribed],
                    return_options: vec![],
                },
            ),
            (
                "A002 LSUB \"#news.\" \"comp.%\"\r\n",
                list::Arguments::Extended {
                    tag: "A002".to_string(),
                    reference_name: "#news.".to_string(),
                    mailbox_name: vec!["comp.%".to_string()],
                    selection_options: vec![SelectionOption::Subscribed],
                    return_options: vec![],
                },
            ),
        ] {
            assert_eq!(
                receiver
                    .parse(&mut command.as_bytes().iter())
                    .unwrap()
                    .parse_lsub()
                    .unwrap(),
                arguments
            );
        }
    }
}