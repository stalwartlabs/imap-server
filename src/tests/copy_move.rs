use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Select INBOX
    imap.send("SELECT INBOX").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    // Copying to "All Mail" or the same mailbox should fail
    imap.send("COPY * INBOX").await;
    imap.assert_read(Type::Tagged, ResponseType::No)
        .await
        .assert_response_code("CANNOT");

    imap.send("COPY * \"All Mail\"").await;
    imap.assert_read(Type::Tagged, ResponseType::No)
        .await
        .assert_response_code("CANNOT");

    // Copying to a non-existent mailbox should fail
    imap.send("COPY * \"/dev/null\"").await;
    imap.assert_read(Type::Tagged, ResponseType::No)
        .await
        .assert_response_code("TRYCREATE");

    // Create test folders
    imap.send("CREATE \"Scamorza Affumicata\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    imap.send("CREATE \"Burrata al Tartufo\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    // Copy messages
    imap.send("COPY 1,3,5,7 \"Scamorza Affumicata\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("COPYUID")
        .assert_contains("0:3");

    // Check status
    imap.send("STATUS \"Scamorza Affumicata\" (UIDNEXT MESSAGES UNSEEN SIZE)")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("MESSAGES 4")
        .assert_contains("UNSEEN 4")
        .assert_contains("UIDNEXT 4")
        .assert_contains("SIZE 5851");

    // Move all messages to Burrata
    imap.send("SELECT \"Scamorza Affumicata\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    imap.send("MOVE * \"Burrata al Tartufo\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("* OK [COPYUID")
        .assert_contains("0:3");

    // Check status
    imap.send("LIST \"\" % RETURN (STATUS (UIDNEXT MESSAGES UNSEEN SIZE))")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("\"Burrata al Tartufo\" (UIDNEXT 4 MESSAGES 4 UNSEEN 4 SIZE 5851)")
        .assert_contains("\"Scamorza Affumicata\" (UIDNEXT 4 MESSAGES 0 UNSEEN 0 SIZE 0)")
        .assert_contains("\"INBOX\" (UIDNEXT 10 MESSAGES 10 UNSEEN 10 SIZE 12193)");

    // Move the messages back to Scamorza, UIDNEXT should increase.
    imap.send("SELECT \"Burrata al Tartufo\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    imap.send("MOVE * \"Scamorza Affumicata\"").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("* OK [COPYUID")
        .assert_contains("4:7");

    // Check status
    imap.send("LIST \"\" % RETURN (STATUS (UIDNEXT MESSAGES UNSEEN SIZE))")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("\"Burrata al Tartufo\" (UIDNEXT 4 MESSAGES 0 UNSEEN 0 SIZE 0)")
        .assert_contains("\"Scamorza Affumicata\" (UIDNEXT 8 MESSAGES 4 UNSEEN 4 SIZE 5851)")
        .assert_contains("\"INBOX\" (UIDNEXT 10 MESSAGES 10 UNSEEN 10 SIZE 12193)");
}
