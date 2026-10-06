use clap::Parser;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::io::{self, Write};
use std::time::Duration;

#[derive(Parser)]

struct Args {
    #[arg(long, default_value = "http://127.0.0.1:7878")]
    url: String,
}

fn input(prompt: &str) -> io::Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

fn handle_response(command: &str, status: u16, value: &Value, token: &mut String) {
    println!("{status} {value}");
    if command == "login"
        && status == 200
        && let Some(next) = value["data"]["token"].as_str()
    {
        *token = next.into();
    }
    if status == 401 {
        println!("Please log in again.");
    }
    if status == 401 || (matches!(command, "logout" | "delete-user") && status == 200) {
        token.clear();
    }
}

fn input_text() -> io::Result<String> {
    println!("Enter text: '.' ends; '.end' ends without the final newline.");
    println!("For a text line starting with '.', add one extra '.' prefix.");
    rm_client_sync::read_multiline(&mut io::stdin().lock())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let client = Client::builder()
        .timeout(Duration::from_secs(12))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut token = String::new();
    loop {
        let command = match input(
            "ping / register / login / logout / list / echo / delete-user / put / get / delete / q > ",
        ) {
            Ok(command) => command,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        };
        let mut body = Value::Null;
        let text_path;
        let (method, path) = match command.as_str() {
            "q" => break,
            "ping" => ("GET", "/ping"),
            "list" => ("GET", "/texts"),
            "logout" => ("DELETE", "/sessions/current"),
            "delete-user" => ("DELETE", "/users/me"),
            "register" | "login" => {
                body = json!({"username": input("username: ")?, "password": rpassword::prompt_password("password: ")?});
                (
                    "POST",
                    if command == "register" {
                        "/users"
                    } else {
                        "/sessions"
                    },
                )
            }
            "echo" => {
                let text = match input_text() {
                    Ok(text) => text,
                    Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(error.into()),
                };
                body = json!({"text": text});
                ("POST", "/echo")
            }
            "put" | "get" | "delete" => {
                let name = input("name: ")?;
                if name.is_empty()
                    || name.len() > 64
                    || !name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    println!("Name must be 1-64 ASCII letters, digits, '_' or '-'.");
                    continue;
                }
                text_path = format!("/texts/{name}");
                if command == "put" {
                    let text = match input_text() {
                        Ok(text) => text,
                        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                        Err(error) => return Err(error.into()),
                    };
                    body = json!({"text": text});
                    ("PUT", text_path.as_str())
                } else if command == "get" {
                    ("GET", text_path.as_str())
                } else {
                    ("DELETE", text_path.as_str())
                }
            }
            _ => {
                println!("Unknown command.");
                continue;
            }
        };
        let result: Result<(u16, Value), reqwest::Error> = rm_client_sync::exchange(
            &client,
            &args.url,
            method.parse().unwrap(),
            path,
            &token,
            if body.is_null() { None } else { Some(&body) },
        );
        match result {
            Ok((status, value)) => {
                handle_response(&command, status, &value, &mut token);
            }
            Err(error) => eprintln!("Request failed: {error}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_deletion_updates_login_state_only_when_confirmed() {
        let mut token = String::new();
        handle_response(
            "login",
            200,
            &json!({"data": {"token": "sample"}}),
            &mut token,
        );
        assert_eq!(token, "sample");
        handle_response(
            "delete-user",
            500,
            &json!({"message": "Failed"}),
            &mut token,
        );
        assert_eq!(token, "sample");
        handle_response("delete", 200, &json!({"data": null}), &mut token);
        assert_eq!(token, "sample");
        handle_response("delete-user", 200, &json!({"data": null}), &mut token);
        assert!(token.is_empty());
        handle_response(
            "login",
            200,
            &json!({"data": {"token": "next"}}),
            &mut token,
        );
        handle_response("delete-user", 401, &Value::Null, &mut token);
        assert!(token.is_empty());
    }
}
