use crate::core::ResponseType;

use super::{
    append::{assert_append_message, build_messages},
    AssertResult, ImapConnection, Type,
};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Create test messages
    let messages = build_messages();

    // Insert messages
    imap.send("CREATE Manchego").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    for message in messages {
        assert_append_message(imap, "Manchego", &message, ResponseType::Ok).await;
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
