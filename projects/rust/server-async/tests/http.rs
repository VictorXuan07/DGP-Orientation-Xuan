use rm_server_async::http::create_app;
use rocket::http::{ContentType, Header, Status};
use rocket::local::blocking::Client;
use serde_json::{Value, json};

#[test]
fn http_echo_preserves_text_and_checks_input() {
    let client = Client::tracked(create_app()).unwrap();
    for text in [
        "".to_owned(),
        " 你好\nRM\n ".to_owned(),
        "😀".repeat(16_384),
    ] {
        let response = client
            .post("/echo")
            .header(ContentType::JSON)
            .body(json!({"text": text}).to_string())
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.content_type(), Some(ContentType::JSON));
        assert_eq!(
            response.into_json::<Value>().unwrap(),
            json!({"data": text})
        );
    }
    for body in [
        "not JSON",
        "[]",
        "{}",
        r#"{"text":42}"#,
        r#"{"text":"hello","extra":true}"#,
        r#"{"text":"\uD800"}"#,
    ] {
        assert_eq!(
            client
                .post("/echo")
                .header(ContentType::JSON)
                .body(body)
                .dispatch()
                .status(),
            Status::BadRequest,
            "{body}"
        );
    }
    assert_eq!(
        client
            .post("/echo")
            .header(ContentType::JSON)
            .body(json!({"text": "a".repeat(65_537)}).to_string())
            .dispatch()
            .status(),
        Status::PayloadTooLarge
    );
    // JSON escapes enlarge the wire body without enlarging the decoded text.
    let escaped = format!(r#"{{"text":"{}"}}"#, "\\u0000".repeat(65_536));
    let exact = format!("{escaped}{}", " ".repeat(524_288 - escaped.len()));
    let response = client
        .post("/echo")
        .header(ContentType::JSON)
        .body(&exact)
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(
        response.into_json::<Value>().unwrap(),
        json!({"data": "\0".repeat(65_536)})
    );
    assert_eq!(
        client
            .post("/echo")
            .header(ContentType::JSON)
            .body(format!("{exact} "))
            .dispatch()
            .status(),
        Status::PayloadTooLarge
    );
    // A rejected request must not prevent subsequent requests from succeeding.
    assert_eq!(client.get("/ping").dispatch().status(), Status::Ok);
}

#[test]
fn http_account_lifecycle() {
    let client = Client::tracked(create_app()).unwrap();
    let ping = client.get("/ping").dispatch();
    assert_eq!(ping.status(), Status::Ok);
    assert_eq!(ping.into_json::<Value>().unwrap(), json!({"data": "pong"}));
    let account = json!({"username": "alice", "password": "password1"}).to_string();
    assert_eq!(
        client
            .post("/users")
            .header(ContentType::JSON)
            .body(&account)
            .dispatch()
            .status(),
        Status::Created
    );
    let login = client
        .post("/sessions")
        .header(ContentType::JSON)
        .body(&account)
        .dispatch()
        .into_json::<Value>()
        .unwrap();
    let authorization = format!("Bearer {}", login["data"]["token"].as_str().unwrap());
    let texts = client
        .get("/texts")
        .header(Header::new("Authorization", authorization.clone()))
        .dispatch();
    assert_eq!(texts.status(), Status::Ok);
    assert_eq!(texts.into_json::<Value>().unwrap(), json!({"data": []}));
    assert_eq!(
        client.get("/texts").dispatch().status(),
        Status::Unauthorized
    );
    assert_eq!(
        client
            .delete("/sessions/current")
            .header(Header::new("Authorization", authorization.clone()))
            .dispatch()
            .status(),
        Status::Ok
    );
    assert_eq!(
        client
            .get("/texts")
            .header(Header::new("Authorization", authorization))
            .dispatch()
            .status(),
        Status::Unauthorized
    );
}

#[test]
fn http_input_and_routing() {
    let client = Client::tracked(create_app()).unwrap();
    for body in [b"not JSON".to_vec(), vec![0xff], b"NaN".to_vec()] {
        assert_eq!(
            client
                .post("/users")
                .header(ContentType::JSON)
                .body(body)
                .dispatch()
                .status(),
            Status::BadRequest
        );
    }
    let exact = format!("{{}}{}", " ".repeat(524_288 - 2));
    assert_eq!(
        client
            .post("/users")
            .header(ContentType::JSON)
            .body(&exact)
            .dispatch()
            .status(),
        Status::BadRequest
    );
    assert_eq!(
        client
            .post("/users")
            .header(ContentType::JSON)
            .body(format!("{exact} "))
            .dispatch()
            .status(),
        Status::PayloadTooLarge
    );
    assert_eq!(
        client
            .post("/users")
            .header(ContentType::JSON)
            .body(r#"{"username":true,"password":"password1"}"#)
            .dispatch()
            .status(),
        Status::BadRequest
    );
    assert_eq!(client.get("/missing").dispatch().status(), Status::NotFound);
    assert_eq!(
        client.get("/echo").dispatch().status(),
        Status::MethodNotAllowed
    );
    assert_eq!(
        client.patch("/ping").dispatch().status(),
        Status::MethodNotAllowed
    );
}

#[test]
fn unimplemented_routes_are_absent() {
    use rocket::http::Method;
    let client = Client::tracked(create_app()).unwrap();
    for (method, path) in [
        (Method::Delete, "/users/me"),
        (Method::Put, "/texts/note"),
        (Method::Get, "/texts/note"),
        (Method::Delete, "/texts/note"),
    ] {
        assert_eq!(
            client.req(method, path).dispatch().status(),
            Status::NotFound
        );
    }
    for path in [
        "/ping",
        "/echo",
        "/users",
        "/sessions",
        "/sessions/current",
        "/texts",
    ] {
        assert_eq!(
            client.patch(path).dispatch().status(),
            Status::MethodNotAllowed
        );
    }
}
