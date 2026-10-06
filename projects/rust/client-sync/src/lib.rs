use reqwest::{Method, blocking::Client};
use serde_json::Value;
use std::io::{self, BufRead};

/// `.` ends input; `.end` also removes the final line ending.
/// Prefix a dot-starting body line with an extra dot.
pub fn read_multiline(reader: &mut impl BufRead) -> io::Result<String> {
    let mut text = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let command = line.strip_suffix('\n').unwrap_or(&line);
        let command = command.strip_suffix('\r').unwrap_or(command);
        match command {
            "." => return Ok(text),
            ".end" => {
                if text.ends_with('\n') {
                    text.pop();
                    if text.ends_with('\r') {
                        text.pop();
                    }
                }
                return Ok(text);
            }
            _ => {
                if line.starts_with("..") {
                    text.push_str(&line[1..]);
                } else {
                    text.push_str(&line);
                }
            }
        }
    }
}

/// Preserve HTTP status even when the error body is not JSON.
pub fn exchange(
    client: &Client,
    url: &str,
    method: Method,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> Result<(u16, Value), reqwest::Error> {
    let mut request = client.request(method, format!("{}{path}", url.trim_end_matches('/')));
    if !token.is_empty() {
        request = request.bearer_auth(token);
    }
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send()?;
    let status = response.status().as_u16();
    let text = response.text()?;
    let value =
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::json!({"message": text}));
    Ok((status, value))
}
