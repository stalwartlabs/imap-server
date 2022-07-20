use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

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

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Create test messages
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

    // Insert messages
    imap.send("CREATE Manchego").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    for message in messages {
        imap.send(&format!("APPEND Manchego {{{}}}", message.len()))
            .await;
        imap.assert_read(Type::Continuation, ResponseType::Ok).await;
        imap.send_untagged(&message).await;
        imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    }

    imap.send("SELECT Manchego").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    // 4 different threads are expected
    imap.send("THREAD REFERENCES UTF-8 *").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("(1 2 3 4)")
        .assert_contains("(5 6 7 8)")
        .assert_contains("(9 10 11 12)");

    imap.send("THREAD REFERENCES UTF-8 SUBJECT T1").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("(5 6 7 8)")
        .assert_count("(1 2 3 4)", 0)
        .assert_count("(9 10 11 12)", 0);

    // Delete all messages
    imap.send("STORE * +FLAGS.SILENT (\\Deleted)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    imap.send("EXPUNGE").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_count("EXPUNGE", 13);
}
