use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(20);
    listener.set_nonblocking(true).unwrap();
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "client did not connect");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("accept failed: {error}"),
        }
    }
}

fn request(stream: TcpStream) -> BufReader<TcpStream> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.starts_with("GET /"), "{line}");
    loop {
        line.clear();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            return reader;
        }
    }
}

fn run(url: &str, commands: &[u8]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rm-client-sync"))
        .args(["--url", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(commands).unwrap();
    child.wait_with_output().unwrap()
}

fn respond(reader: &mut BufReader<TcpStream>, status: &str, body: &str) {
    write!(
        reader.get_mut(),
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
}

#[test]
fn empty_and_non_json_errors_show_status_and_allow_next_command() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        for (status, body) in [
            ("401 Unauthorized", ""),
            ("500 Internal Server Error", "<html>failed</html>"),
            ("200 OK", "{\"data\":\"pong\"}"),
        ] {
            respond(&mut request(accept(&listener)), status, body);
        }
    });
    let output = run(&url, b"list\nping\nping\nq\n");
    peer.join().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("401 {\"message\":\"\"}"));
    assert!(stdout.contains("Please log in again."));
    assert!(stdout.contains("500"));
    assert!(stdout.contains("<html>failed</html>"));
    assert!(stdout.contains("200 {\"data\":\"pong\"}"));
}

#[test]
fn connection_refused_reports_error_and_accepts_another_command() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let output = run(&url, b"ping\nping\nq\n");
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.matches("Request failed:").count(), 2);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.matches(" / q > ").count(), 3);
}

#[test]
fn production_timeout_reports_error_and_next_request_succeeds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = std::thread::spawn(move || {
        // Keep the first connection open without returning any HTTP response.
        let stalled = request(accept(&listener));
        respond(
            &mut request(accept(&listener)),
            "200 OK",
            "{\"data\":\"pong\"}",
        );
        drop(stalled);
    });
    let started = Instant::now();
    let output = run(&url, b"ping\nping\nq\n");
    peer.join().unwrap();
    assert!(started.elapsed() >= Duration::from_secs(12));
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr.matches("Request failed:").count(), 1);
    assert!(String::from_utf8(output.stdout).unwrap().contains("pong"));
}

#[test]
fn stalled_response_returns_a_timeout_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (release, wait) = std::sync::mpsc::channel();
    let peer = std::thread::spawn(move || {
        let stalled = request(accept(&listener));
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(stalled);
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    let error = rm_client_sync::exchange(&client, &url, reqwest::Method::GET, "/ping", "", None)
        .unwrap_err();
    release.send(()).unwrap();
    peer.join().unwrap();
    assert!(error.is_timeout(), "{error:?}");
}

#[test]
fn invalid_arguments_are_reported_and_eof_exits() {
    let output = Command::new(env!("CARGO_BIN_EXE_rm-client-sync"))
        .arg("--unknown-option")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr).unwrap().contains("error:"));
    assert!(run("http://127.0.0.1:1", b"").status.success());
}
