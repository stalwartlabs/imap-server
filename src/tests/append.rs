use std::{fs, io, path::PathBuf};

use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Invalid APPEND commands
    imap.send("APPEND \"All Mail\" {1+}\r\na").await;
    imap.assert_read(Type::Tagged, ResponseType::No)
        .await
        .assert_response_code("CANNOT");
    imap.send("APPEND \"Does not exist\" {1+}\r\na").await;
    imap.assert_read(Type::Tagged, ResponseType::No)
        .await
        .assert_response_code("TRYCREATE");

    // Import test messages
    let mut test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    test_dir.push("src");
    test_dir.push("tests");
    test_dir.push("resources");
    test_dir.push("messages");

    let mut entries = fs::read_dir(&test_dir)
        .unwrap()
        .map(|res| res.map(|e| e.path()))
        .collect::<Result<Vec<_>, io::Error>>()
        .unwrap();

    entries.sort();

    let mut expected_uid = 0;
    for file_name in entries {
        if file_name.extension().map_or(true, |e| e != "txt") {
            continue;
        }
        let raw_message = fs::read(&file_name).unwrap();

        imap.send(&format!(
            "APPEND INBOX (Flag_{}) {{{}}}",
            file_name
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .split_once('.')
                .unwrap()
                .0,
            raw_message.len()
        ))
        .await;
        imap.assert_read(Type::Continuation, ResponseType::Ok).await;
        imap.send_untagged(std::str::from_utf8(&raw_message).unwrap())
            .await;
        let result = imap
            .assert_read(Type::Tagged, ResponseType::Ok)
            .await
            .into_response_code();
        let mut code = result.split(' ');
        assert_eq!(code.next(), Some("APPENDUID"));
        assert_ne!(code.next(), Some("0"));
        assert_eq!(code.next(), Some(expected_uid.to_string().as_str()));
        expected_uid += 1;
    }
}

pub async fn assert_append_message(imap: &mut ImapConnection, folder: &str, message: &str) {
    imap.send(&format!("APPEND \"{}\" {{{}}}", folder, message.len()))
        .await;
    imap.assert_read(Type::Continuation, ResponseType::Ok).await;
    imap.send_untagged(message).await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
}

fn build_message(message: usize, in_reply_to: Option<usize>, thread_num: usize) -> String {
    if let Some(in_reply_to) = in_reply_to {
        format!(
            "Message-ID: <{}@domain>\nReferences: <{}@domain>\nSubject: re: T{}\n\nreply\n",
            message, in_reply_to, thread_num
        )
    } else {
        format!(
            "Message-ID: <{}@domain>\nSubject: T{}\n\nmsg\n",
            message, thread_num
        )
    }
}

pub fn build_messages() -> Vec<String> {
    let mut messages = Vec::new();
    for parent in 0..3 {
        messages.push(build_message(parent, None, parent));
        for child in 0..3 {
            messages.push(build_message(
                ((parent + 1) * 10) + child,
                parent.into(),
                parent,
            ));
        }
    }
    messages
}
