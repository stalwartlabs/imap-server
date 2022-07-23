use crate::core::ResponseType;

use super::{AssertResult, ImapConnection, Type};

pub async fn test(imap: &mut ImapConnection, _imap_check: &mut ImapConnection) {
    // Examine INBOX
    imap.send("EXAMINE INBOX").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("10 EXISTS")
        .assert_contains("[UIDNEXT 10]");

    // Fetch all properties available from JMAP
    imap.send(concat!(
        "FETCH 10 (FLAGS INTERNALDATE PREVIEW EMAILID THREADID ",
        "RFC822.SIZE UID ENVELOPE BODYSTRUCTURE)"
    ))
    .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("FLAGS (Flag_009)")
        .assert_contains("RFC822.SIZE 1457")
        .assert_contains("UID 9")
        .assert_contains("INTERNALDATE")
        .assert_contains("THREADID (")
        .assert_contains("EMAILID (")
        .assert_contains("but then I thought, why not do both?")
        .assert_contains(concat!(
            "ENVELOPE (\"Sat, 20 Nov 2021 22:22:01 +0000\" ",
            "\"Why not both importing AND exporting? ☺\" ",
            "((\"Art Vandelay (Vandelay Industries)\" NIL \"art\" \"vandelay.com\")) ",
            "NIL NIL ",
            "((NIL NIL \"Colleagues\" NIL) ",
            "(\"James Smythe\" NIL \"james\" \"vandelay.com\") ",
            "(NIL NIL NIL NIL) ",
            "(NIL NIL \"Friends\" NIL) ",
            "(NIL NIL \"jane\" \"example.com\") ",
            "(\"John Smîth\" NIL \"john\" \"example.com\") ",
            "(NIL NIL NIL NIL)) ",
            "NIL NIL NIL NIL)"
        ))
        .assert_contains(concat!(
            "BODYSTRUCTURE ((\"TEXT\" \"html\" (\"charset\" \"us-ascii\") ",
            "NIL NIL \"base64\" 240 4 \"bbd48b00b757cefb9bb72d53a4cdb531\" NIL NIL NIL)",
            " (\"MESSAGE\" \"rfc822\" NIL NIL NIL NIL 724 ",
            "(NIL \"Exporting my book about coffee tables\" ",
            "((\"Cosmo Kramer\" NIL \"kramer\" \"kramerica.com\")) ",
            "NIL NIL NIL NIL NIL NIL NIL) ",
            "(\"image\" \"gif\" (\"name\" \"Book about ☕ tables.gif\") ",
            "NIL NIL \"Base64\" 57 \"753818204af22e11ee8cefb153004fc7\" ",
            "(\"attachment\" ()) NIL NIL) 0 ",
            "\"fea0d696b570c4904fa45f401566fc57\" NIL NIL NIL) ",
            "\"mixed\" (\"boundary\" \"festivus\") NIL NIL NIL)"
        ));

    // Fetch bodyparts
    imap.send(concat!(
        "UID FETCH 9 (BINARY[1] BINARY.SIZE[1] BODY[1.TEXT] BODY[2.1.HEADER] ",
        "BINARY[2.1] BODY[MIME] BODY[HEADER.FIELDS (From)]<11.19>)"
    ))
    .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("BINARY[1] {175}")
        .assert_contains("BINARY.SIZE[1] 175")
        .assert_contains("BODY[1.TEXT] {240}")
        .assert_contains("BODY[2.1.HEADER] {88}")
        .assert_contains("BINARY[2.1] {101}")
        .assert_contains("BODY[MIME] {53}")
        .assert_contains("BODY[HEADER.FIELDS (From)]<11> {8}")
        .assert_contains("&ldquo;exporting&rdquo;")
        .assert_contains("PGh0bWw+PHA+")
        .assert_contains("Content-Transfer-Encoding: quoted-printable")
        .assert_contains("ℌ𝔢𝔩𝔭 𝔪𝔢 𝔢𝔵𝔭𝔬𝔯𝔱 𝔪𝔶 𝔟𝔬𝔬𝔨")
        .assert_contains("Vandelay");

    // We are in EXAMINE mode, fetching body should not set \Seen
    imap.send("UID FETCH 9 (FLAGS)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("FLAGS (Flag_009)");

    // Switch to SELECT mode
    imap.send("SELECT INBOX").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;

    // Peek bodyparts
    imap.send("UID FETCH 9 (BINARY.PEEK[1] BINARY.SIZE[1] BODY.PEEK[1.TEXT])")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("BINARY[1] {175}")
        .assert_contains("BINARY.SIZE[1] 175")
        .assert_contains("BODY[1.TEXT] {240}");

    // PEEK was used, \Seen should not be set
    imap.send("UID FETCH 9 (FLAGS)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("FLAGS (Flag_009)");

    // Fetching a body section should set the \Seen flag
    imap.send("UID FETCH 9 (BODY[1.TEXT])").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("FLAGS")
        .assert_contains("\\Seen");

    // Fetch a sequence
    imap.send("FETCH 1:5,7:10 (UID FLAGS)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("* 1 FETCH (UID 0 ")
        .assert_contains("* 2 FETCH (UID 1 ")
        .assert_contains("* 3 FETCH (UID 2 ")
        .assert_contains("* 4 FETCH (UID 3 ")
        .assert_contains("* 5 FETCH (UID 4 ")
        .assert_contains("* 7 FETCH (UID 6 ")
        .assert_contains("* 8 FETCH (UID 7 ")
        .assert_contains("* 9 FETCH (UID 8 ")
        .assert_contains("* 10 FETCH (UID 9 ");

    imap.send("FETCH 7:* (UID FLAGS)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("* 7 FETCH (UID 6 ")
        .assert_contains("* 8 FETCH (UID 7 ")
        .assert_contains("* 9 FETCH (UID 8 ")
        .assert_contains("* 10 FETCH (UID 9 ");

    // Fetch using a saved search
    imap.send("UID SEARCH RETURN (SAVE) FROM \"nathaniel\"")
        .await;
    imap.assert_read(Type::Tagged, ResponseType::Ok).await;
    imap.send("FETCH $ (UID PREVIEW)").await;
    imap.assert_read(Type::Tagged, ResponseType::Ok)
        .await
        .assert_contains("* 1 FETCH (UID 0 ")
        .assert_contains("* 4 FETCH (UID 3 ")
        .assert_contains("* 6 FETCH (UID 5 ")
        .assert_contains("Some text appears here")
        .assert_contains("plain text version of message goes here")
        .assert_contains("This is implicitly typed plain US-ASCII text.");
}
