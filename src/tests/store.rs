use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Select INBOX
    imap.send("SELECT INBOX").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("10 EXISTS")
        .assert_contains("[UIDNEXT 10]");

    // Set all messages to flag "Seen"
    imap.send("UID STORE * +FLAGS.SILENT (\\Seen)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_count("FLAGS", 0);

    // Check that the flags were set
    imap.send("UID FETCH * (Flags)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_count("\\Seen", 10);

    // Check status
    imap.send("STATUS INBOX (UIDNEXT MESSAGES UNSEEN)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("MESSAGES 10")
        .assert_contains("UNSEEN 0")
        .assert_contains("UIDNEXT 10");

    // Remove Seen flag from all messages
    imap.send("UID STORE * -FLAGS (\\Seen)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_count("FLAGS", 10)
        .assert_count("Seen", 0);

    // Store using saved searches
    imap.send("SEARCH RETURN (SAVE) FROM nathaniel").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    imap.send("UID STORE $ +FLAGS (\\Answered)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_count("FLAGS", 3);

    // Remove Answered flag
    imap.send("UID STORE * -FLAGS (\\Answered)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_count("FLAGS", 10)
        .assert_count("Answered", 0);
}
