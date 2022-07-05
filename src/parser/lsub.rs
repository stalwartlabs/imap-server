use crate::{
    core::receiver::Request,
    protocol::list::{self, SelectionOption},
};

pub fn parse_lsub(request: Request) -> crate::core::Result<list::Arguments> {
    if request.tokens.len() > 1 {
        let mut tokens = request.tokens.into_iter();

        Ok(list::Arguments::Extended {
            reference_name: tokens
                .next()
                .ok_or((request.tag.as_str(), "Missing reference name."))?
                .unwrap_string()
                .map_err(|v| (request.tag.as_str(), v))?,
            mailbox_name: vec![tokens
                .next()
                .ok_or((request.tag.as_str(), "Missing mailbox name."))?
                .unwrap_string()
                .map_err(|v| (request.tag.as_str(), v))?],
            selection_options: vec![SelectionOption::Subscribed],
            return_options: vec![],
            tag: request.tag,
        })
    } else {
        Err(request.into_error("Missing arguments."))
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
                super::parse_lsub(receiver.parse(&mut command.as_bytes().iter()).unwrap()).unwrap(),
                arguments
            );
        }
    }
}
