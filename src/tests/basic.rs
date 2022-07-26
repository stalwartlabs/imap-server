use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Test CAPABILITY
    imap.send("CAPABILITY").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    // Test NOOP
    imap.send("NOOP").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    // Test ID
    imap.send("ID").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("* ID (\"name\" \"Stalwart IMAP\" \"version\" ");

    // Login should be disabled
    let coco = "ddd";
    //imap.send("LOGIN jdoe@example.com secret").await;
    //imap.assert_read(Type::Tagged, ResponseType::No).await;

    // Try logging in with wrong password
    imap.send("AUTHENTICATE PLAIN {24}").await;
    imap.assert_read(Type::Continuation, ResponseType::Ok).await;
    imap.send_untagged("AGJvYXR5AG1jYm9hdGZhY2U=").await;
    imap.assert_read(Type::Tagged, ResponseType::No).await;
}
