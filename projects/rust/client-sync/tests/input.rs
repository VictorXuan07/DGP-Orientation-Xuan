use rm_client_sync::read_multiline;
use std::io::{self, BufRead, Cursor};

#[test]
fn multiline_preserves_content_and_supports_both_endings() {
    for (input, expected) in [
        (".\n", ""),
        (".end\n", ""),
        ("你好\nRM\n.\n", "你好\nRM\n"),
        ("你好\nRM\n.end\n", "你好\nRM"),
        ("\n\n.\n", "\n\n"),
        ("\n\n.end\n", "\n"),
        ("  spaces  \n.end\n", "  spaces  "),
        ("..\n..end\n...abc\n.\n", ".\n.end\n..abc\n"),
        ("a\r\nb\r\n.\r\n", "a\r\nb\r\n"),
        ("a\r\nb\r\n.end\r\n", "a\r\nb"),
    ] {
        assert_eq!(read_multiline(&mut Cursor::new(input)).unwrap(), expected);
    }
}

#[test]
fn ending_leaves_the_next_command_unread() {
    let mut reader = Cursor::new("hello\n.end\nping\n");
    assert_eq!(read_multiline(&mut reader).unwrap(), "hello");
    let mut next = String::new();
    reader.read_line(&mut next).unwrap();
    assert_eq!(next, "ping\n");
}

#[test]
fn missing_ending_or_invalid_utf8_reports_an_error() {
    for bytes in [b"".as_slice(), b"unfinished\n", b"unfinished"] {
        assert_eq!(
            read_multiline(&mut Cursor::new(bytes)).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
    assert_eq!(
        read_multiline(&mut Cursor::new([0xff, b'\n']))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}
