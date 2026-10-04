use rm_client_sync::read_multiline;
use std::io::{self, BufRead, Cursor};

#[test]
fn preserves_text_and_escapes_markers() {
    for (input, expected) in [
        (".\n", ""),
        (".end\n", ""),
        ("你好\nRM\n.\n", "你好\nRM\n"),
        ("你好\nRM\n.end\n", "你好\nRM"),
        ("\n\n.\n", "\n\n"),
        ("  text  \n.end\n", "  text  "),
        ("..\n..end\n...abc\n.\n", ".\n.end\n..abc\n"),
        ("你好\r\nRM\r\n.end\r\n", "你好\r\nRM"),
    ] {
        assert_eq!(read_multiline(&mut Cursor::new(input)).unwrap(), expected);
    }
}

#[test]
fn leaves_next_command_unread() {
    let mut reader = Cursor::new("hello\n.\nping\n");
    assert_eq!(read_multiline(&mut reader).unwrap(), "hello\n");
    let mut command = String::new();
    reader.read_line(&mut command).unwrap();
    assert_eq!(command, "ping\n");
}

#[test]
fn unfinished_input_reports_eof() {
    assert_eq!(
        read_multiline(&mut Cursor::new("unfinished\n"))
            .unwrap_err()
            .kind(),
        io::ErrorKind::UnexpectedEof
    );
}
