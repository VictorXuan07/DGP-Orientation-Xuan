use reqwest::{Method, blocking::Client};
use rm_client_sync::exchange;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Duration;

#[test]
fn delete_user_command_sends_no_body_and_keeps_command_loop_running() {
    use std::process::{Command, Stdio};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        for (method, path, status, response) in [
            ("DELETE", "/users/me", "200 OK", json!({"data": null})),
            (
                "DELETE",
                "/users/me",
                "401 Unauthorized",
                json!({"message": "Login required"}),
            ),
            ("GET", "/ping", "200 OK", json!({"data": "pong"})),
        ] {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, format!("{method} {path} HTTP/1.1\r\n"));
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                let header = line.to_ascii_lowercase();
                if let Some(value) = header.strip_prefix("content-length:") {
                    assert_eq!(value.trim(), "0");
                }
                assert!(!header.starts_with("transfer-encoding:"));
                assert!(!header.starts_with("authorization:"));
            }
            let response = response.to_string();
            write!(reader.get_mut(), "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_rm-client-sync"))
        .args(["--url", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"delete-user\ndelete-user\nping\nq\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    peer.join().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("200 {\"data\":null}"));
    assert!(stdout.contains("401"));
    assert!(stdout.contains("Please log in again."));
    assert!(stdout.contains("pong"));
    assert!(!stdout.contains("Unknown command"));
}

#[test]
fn get_and_delete_commands_send_no_body_and_continue_after_errors() {
    use std::process::{Command, Stdio};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        for (method, name, status, response) in [
            ("GET", "note", "200 OK", json!({"data": "你好\nRM"})),
            ("GET", "empty", "200 OK", json!({"data": ""})),
            (
                "GET",
                "missing",
                "404 Not Found",
                json!({"message": "Text not found"}),
            ),
            (
                "GET",
                "note",
                "401 Unauthorized",
                json!({"message": "Login required"}),
            ),
            ("DELETE", "note", "200 OK", json!({"data": null})),
            (
                "DELETE",
                "note",
                "404 Not Found",
                json!({"message": "Text not found"}),
            ),
            (
                "DELETE",
                "note",
                "401 Unauthorized",
                json!({"message": "Login required"}),
            ),
        ] {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(line, format!("{method} /texts/{name} HTTP/1.1\r\n"));
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
                let header = line.to_ascii_lowercase();
                if let Some(value) = header.strip_prefix("content-length:") {
                    assert_eq!(value.trim(), "0");
                }
                assert!(!header.starts_with("transfer-encoding:"));
            }
            let response = response.to_string();
            write!(reader.get_mut(), "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_rm-client-sync"))
        .args(["--url", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("get\nbad/name\nget\nnote\nget\nempty\nget\nmissing\nget\nnote\ndelete\nbad/name\ndelete\nnote\ndelete\nnote\ndelete\nnote\nq\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    peer.join().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Name must be 1-64"));
    assert!(stdout.contains(&format!("200 {}", json!({"data": "你好\nRM"}))));
    assert!(stdout.contains(&format!("200 {}", json!({"data": ""}))));
    assert!(stdout.contains(&format!("200 {}", json!({"data": null}))));
    assert!(stdout.contains("404"));
    assert!(stdout.contains("401"));
    assert!(stdout.contains("Please log in again."));
    assert!(!stdout.contains("Unknown command"));
}

#[test]
fn put_command_validates_name_sends_text_and_handles_unauthorized() {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "PUT /texts/note-1 HTTP/1.1\r\n");
        let mut length = None;
        let mut headers = String::new();
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            let header = line.to_ascii_lowercase();
            if let Some(value) = header.strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
            headers.push_str(&header);
        }
        assert!(headers.contains("content-type: application/json\r\n"));
        let mut body = vec![0; length.unwrap()];
        reader.read_exact(&mut body).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({"text": "你好 RM"})
        );
        reader.get_mut().write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: 26\r\nConnection: close\r\n\r\n{\"message\":\"Login needed\"}").unwrap();
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_rm-client-sync"))
        .args(["--url", &url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all("put\nbad/name\nput\nnote-1\n你好 RM\nq\n".as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    peer.join().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Name must be 1-64"));
    assert!(stdout.contains("401"));
    assert!(stdout.contains("Please log in again."));
}

#[test]
fn sends_http_authorization_and_preserves_error_status() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut headers = String::new();
        loop {
            let mut line = String::new();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            headers.push_str(&line);
        }
        assert!(headers.starts_with("GET /texts HTTP/1.1\r\n"));
        assert!(
            headers
                .to_lowercase()
                .contains("authorization: bearer sample\r\n")
        );
        stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 7\r\nConnection: close\r\n\r\nexpired").unwrap();
    });
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let result = exchange(&client, &url, Method::GET, "/texts", "sample", None).unwrap();
    assert_eq!(result, (401, json!({"message":"expired"})));
    peer.join().unwrap();
}

#[test]
fn echo_sends_json_and_reads_original_text() {
    use std::io::Read;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let text = "你好 RM".to_owned();
    let expected = text.clone();
    let peer = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "POST /echo HTTP/1.1\r\n");
        let mut length = None;
        let mut json_content_type = false;
        loop {
            line.clear();
            assert!(reader.read_line(&mut line).unwrap() > 0);
            if line == "\r\n" {
                break;
            }
            let header = line.to_ascii_lowercase();
            if let Some(value) = header.strip_prefix("content-length:") {
                length = Some(value.trim().parse::<usize>().unwrap());
            }
            if header.trim() == "content-type: application/json" {
                json_content_type = true;
            }
        }
        assert!(json_content_type);
        let mut body = vec![0; length.unwrap()];
        reader.read_exact(&mut body).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({"text": expected})
        );
        let response = json!({"data": expected}).to_string();
        write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
    });
    let client = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .unwrap();
    let result = exchange(
        &client,
        &url,
        Method::POST,
        "/echo",
        "",
        Some(&json!({"text": text})),
    )
    .unwrap();
    assert_eq!(result, (200, json!({"data": text})));
    peer.join().unwrap();
}
