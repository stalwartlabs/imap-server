use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Searches without selecting a mailbox should fail.
    imap.send("SEARCH RETURN (MIN MAX COUNT ALL) ALL").await;
    imap.assert_read(Type::Tagged, ResponseType::Bad).await;

    // Select INBOX
    imap.send("SELECT INBOX").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("10 EXISTS")
        .assert_contains("[UIDNEXT 10]");

    // Min, Max and Count
    imap.send("SEARCH RETURN (MIN MAX COUNT ALL) ALL").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("COUNT 10 MIN 1 MAX 10 ALL 1,10");
    imap.send("UID SEARCH ALL").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 0 1 2 3 4 5 6 7 8 9");

    // Filters
    imap.send("UID SEARCH OR FROM nathaniel SUBJECT argentina")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 0 2 3 5");

    imap.send("UID SEARCH UNSEEN OR KEYWORD Flag_007 KEYWORD Flag_004")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 4 7");

    imap.send("UID SEARCH TEXT coffee FROM vandelay SUBJECT exporting SENTON 20-Nov-2021")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 9");

    imap.send("UID SEARCH NOT (FROM nathaniel ANSWERED)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 1 2 4 6 7 8 9");

    imap.send("UID SEARCH UID 0:6 LARGER 1000 SMALLER 2000")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 0 1");

    // Saved search
    imap.send(
        "UID SEARCH RETURN (SAVE ALL) OR OR FROM nathaniel FROM vandelay OR SUBJECT rfc FROM gore",
    )
    .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("0,2:3,5,7,9");

    imap.send("UID SEARCH NOT $").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 1 4 6 8");

    imap.send("UID SEARCH $ SMALLER 1000 SUBJECT section").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SEARCH 7");

    imap.send("UID SEARCH RETURN (MIN MAX) NOT $").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("MIN 1 MAX 8");

    // Sort
    imap.send("UID SORT (REVERSE SUBJECT REVERSE DATE) UTF-8 FROM Nathaniel")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_equals("* SORT 5 3 0");

    imap.send("UID SORT RETURN (COUNT ALL) (DATE SUBJECT) UTF-8 ALL")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("COUNT 10 ALL 5,3:4,0,2,6:7,9,1,8");
}