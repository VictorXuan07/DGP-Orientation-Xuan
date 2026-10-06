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
fn http_put_text_creates_and_overwrites() {
    let client = Client::tracked(create_app()).unwrap();
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

    for text in ["first", "你好\nRM\n", ""] {
        let response = client
            .put("/texts/note")
            .header(ContentType::JSON)
            .header(Header::new("Authorization", authorization.clone()))
            .body(json!({"text": text}).to_string())
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(
            response.into_json::<Value>().unwrap(),
            json!({"data": null})
        );
        let response = client
            .get("/texts/note")
            .header(Header::new("Authorization", authorization.clone()))
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.content_type(), Some(ContentType::JSON));
        assert_eq!(
            response.into_json::<Value>().unwrap(),
            json!({"data": text})
        );
    }
    for (path, expected) in [
        ("/texts/missing", Status::NotFound),
        ("/texts/bad.name", Status::BadRequest),
    ] {
        assert_eq!(
            client
                .get(path)
                .header(Header::new("Authorization", authorization.clone()))
                .dispatch()
                .status(),
            expected
        );
    }
    assert_eq!(
        client.get("/texts/note").dispatch().status(),
        Status::Unauthorized
    );
    let list = client
        .get("/texts")
        .header(Header::new("Authorization", authorization.clone()))
        .dispatch();
    assert_eq!(
        list.into_json::<Value>().unwrap(),
        json!({"data": ["note"]})
    );
    for (path, expected) in [
        ("/texts/bad.name", Status::BadRequest),
        ("/texts/missing", Status::NotFound),
    ] {
        assert_eq!(
            client
                .delete(path)
                .header(Header::new("Authorization", authorization.clone()))
                .dispatch()
                .status(),
            expected
        );
    }
    assert_eq!(
        client.delete("/texts/note").dispatch().status(),
        Status::Unauthorized
    );
    let response = client
        .delete("/texts/note")
        .header(Header::new("Authorization", authorization.clone()))
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.content_type(), Some(ContentType::JSON));
    assert_eq!(
        response.into_json::<Value>().unwrap(),
        json!({"data": null})
    );
    assert_eq!(
        client
            .get("/texts/note")
            .header(Header::new("Authorization", authorization.clone()))
            .dispatch()
            .status(),
        Status::NotFound
    );
    assert_eq!(
        client
            .delete("/texts/note")
            .header(Header::new("Authorization", authorization.clone()))
            .dispatch()
            .status(),
        Status::NotFound
    );
    let response = client
        .get("/texts")
        .header(Header::new("Authorization", authorization))
        .dispatch();
    assert_eq!(response.status(), Status::Ok);
    assert_eq!(response.into_json::<Value>().unwrap(), json!({"data": []}));
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
    let client = Client::tracked(create_app()).unwrap();
    assert_eq!(
        client.get("/not-implemented").dispatch().status(),
        Status::NotFound
    );
    assert_eq!(
        client.delete("/users/me").dispatch().status(),
        Status::Unauthorized
    );
    assert_eq!(
        client.put("/texts/note").dispatch().status(),
        Status::BadRequest
    );
    assert_eq!(
        client.delete("/texts/note").dispatch().status(),
        Status::Unauthorized
    );
    for path in [
        "/ping",
        "/echo",
        "/users",
        "/sessions",
        "/sessions/current",
        "/texts",
        "/texts/note",
        "/users/me",
    ] {
        assert_eq!(
            client.patch(path).dispatch().status(),
            Status::MethodNotAllowed
        );
    }
}

#[test]
fn http_account_deletion_allows_clean_reregistration() {
    let client = Client::tracked(create_app()).unwrap();
    let account = json!({"username": "alice", "password": "password1"}).to_string();
    for generation in 0..2 {
        assert_eq!(
            client
                .post("/users")
                .header(ContentType::JSON)
                .body(&account)
                .dispatch()
                .status(),
            Status::Created
        );
        let response = client
            .post("/sessions")
            .header(ContentType::JSON)
            .body(&account)
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        let login = response.into_json::<Value>().unwrap();
        let token = format!("Bearer {}", login["data"]["token"].as_str().unwrap());
        assert_eq!(
            client
                .get("/texts")
                .header(Header::new("Authorization", token.clone()))
                .dispatch()
                .into_json::<Value>()
                .unwrap(),
            json!({"data": []})
        );
        if generation == 0 {
            assert_eq!(
                client
                    .put("/texts/note")
                    .header(ContentType::JSON)
                    .header(Header::new("Authorization", token.clone()))
                    .body(json!({"text": "hello"}).to_string())
                    .dispatch()
                    .status(),
                Status::Ok
            );
        }
        let response = client
            .delete("/users/me")
            .header(Header::new("Authorization", token.clone()))
            .dispatch();
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.content_type(), Some(ContentType::JSON));
        assert_eq!(
            response.into_json::<Value>().unwrap(),
            json!({"data": null})
        );
        assert_eq!(
            client
                .get("/texts")
                .header(Header::new("Authorization", token.clone()))
                .dispatch()
                .status(),
            Status::Unauthorized
        );
        assert_eq!(
            client
                .delete("/users/me")
                .header(Header::new("Authorization", token))
                .dispatch()
                .status(),
            Status::Unauthorized
        );
    }
}
