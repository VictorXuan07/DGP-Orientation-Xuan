use rm_server_async::http::create_app;
use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::blocking::Client;
use serde_json::{Value, json};

fn request(
    client: &Client,
    method: Method,
    path: &str,
    token: &str,
    body: Option<&Value>,
) -> (Status, Value) {
    let mut request = client.req(method, path);
    if !token.is_empty() {
        request = request.header(Header::new("Authorization", token.to_owned()));
    }
    if let Some(body) = body {
        request = request.header(ContentType::JSON).body(body.to_string());
    }
    let response = request.dispatch();
    let status = response.status();
    if status.code < 400 {
        assert_eq!(response.content_type(), Some(ContentType::JSON));
        (status, response.into_json::<Value>().unwrap())
    } else {
        // The protocol permits missing or framework-native error bodies.
        (status, Value::Null)
    }
}

fn login(client: &Client, account: &Value) -> String {
    let (status, body) = request(client, Method::Post, "/sessions", "", Some(account));
    assert_eq!(status, Status::Ok);
    assert_eq!(body.as_object().unwrap().len(), 1);
    assert_eq!(body["data"].as_object().unwrap().len(), 2);
    assert_eq!(body["data"]["expires_in"], 300);
    let token = body["data"]["token"].as_str().unwrap();
    assert!(!token.is_empty());
    format!("Bearer {token}")
}

fn register(client: &Client, name: &str, password: &str) -> Value {
    let account = json!({"username": name, "password": password});
    assert_eq!(
        request(client, Method::Post, "/users", "", Some(&account)),
        (Status::Created, json!({"data": {"username": name}}))
    );
    account
}

#[test]
fn account_fields_and_unicode_password_boundaries() {
    let client = Client::tracked(create_app()).unwrap();
    let mut invalid = vec![
        Value::Null,
        json!([]),
        json!("account"),
        json!({}),
        json!({"username": "alice"}),
        json!({"password": "password1"}),
        json!({"username": "alice", "password": "password1", "extra": true}),
    ];
    for field in ["username", "password"] {
        for value in [Value::Null, json!(true), json!(42), json!([]), json!({})] {
            let mut account = json!({"username": "alice", "password": "password1"});
            account[field] = value;
            invalid.push(account);
        }
    }
    for name in [
        "".to_owned(),
        "a".repeat(33),
        "a.b".into(),
        "a/b".into(),
        "a b".into(),
        "你好".into(),
    ] {
        invalid.push(json!({"username": name, "password": "password1"}));
    }
    for password in [
        "".to_owned(),
        "😀".repeat(7),
        "a".repeat(129),
        "😀".repeat(129),
    ] {
        invalid.push(json!({"username": "alice", "password": password}));
    }
    for path in ["/users", "/sessions"] {
        for body in &invalid {
            assert_eq!(
                request(&client, Method::Post, path, "", Some(body)).0,
                Status::BadRequest,
                "{path}: {body}"
            );
        }
    }
    // 8 and 128 Unicode scalar values; a combining accent counts separately.
    for (name, password) in [
        ("A".to_owned(), "😀".repeat(8)),
        ("a".repeat(32), "😀".repeat(128)),
        ("a_1-".to_owned(), "e\u{301}".repeat(4)),
        ("a".to_owned(), "password1".to_owned()),
    ] {
        let account = register(&client, &name, &password);
        login(&client, &account);
        assert_eq!(
            request(&client, Method::Post, "/users", "", Some(&account)).0,
            Status::Conflict
        );
    }
    for body in [
        json!({"username": "missing", "password": "password1"}),
        json!({"username": "a", "password": "wrongpass"}),
    ] {
        assert_eq!(
            request(&client, Method::Post, "/sessions", "", Some(&body)).0,
            Status::Unauthorized
        );
    }
    assert_eq!(
        request(&client, Method::Get, "/ping", "", None),
        (Status::Ok, json!({"data": "pong"}))
    );
}

fn assert_identity_rejected(client: &Client, token: &str) {
    for (method, path) in [
        (Method::Get, "/texts"),
        (Method::Get, "/texts/note"),
        (Method::Put, "/texts/note"),
        (Method::Delete, "/texts/note"),
        (Method::Delete, "/sessions/current"),
        (Method::Delete, "/users/me"),
    ] {
        let body = json!({"text": "must not change data"});
        let body = (method == Method::Put).then_some(&body);
        assert_eq!(
            request(client, method, path, token, body).0,
            Status::Unauthorized,
            "{method} {path}: {token}"
        );
    }
}

#[test]
fn all_protected_interfaces_reject_missing_replaced_and_revoked_tokens() {
    let client = Client::tracked(create_app()).unwrap();
    let account = register(&client, "alice", "password1");
    let old = login(&client, &account);
    let saved = json!({"text": "keep"});
    assert_eq!(
        request(&client, Method::Put, "/texts/note", &old, Some(&saved)).0,
        Status::Ok
    );
    for token in ["", "Bearer invalid", "sample", "Bearer "] {
        assert_identity_rejected(&client, token);
    }
    let current = login(&client, &account);
    assert_ne!(old, current);
    assert_identity_rejected(&client, &old);
    let wrong = json!({"username": "alice", "password": "wrongpass"});
    assert_eq!(
        request(&client, Method::Post, "/sessions", "", Some(&wrong)).0,
        Status::Unauthorized
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/note", &current, None),
        (Status::Ok, json!({"data": "keep"}))
    );
    assert_eq!(
        request(&client, Method::Delete, "/sessions/current", &current, None),
        (Status::Ok, json!({"data": null}))
    );
    assert_identity_rejected(&client, &current);
    let deleted = login(&client, &account);
    assert_eq!(
        request(&client, Method::Delete, "/users/me", &deleted, None).0,
        Status::Ok
    );
    let account = register(&client, "alice", "password1");
    let fresh = login(&client, &account);
    assert_identity_rejected(&client, &deleted);
    assert_eq!(
        request(&client, Method::Get, "/texts", &fresh, None),
        (Status::Ok, json!({"data": []}))
    );
}

#[test]
fn text_fields_names_and_utf8_limits_reject_without_overwriting() {
    let client = Client::tracked(create_app()).unwrap();
    let account = register(&client, "alice", "password1");
    let token = login(&client, &account);
    assert_eq!(
        request(
            &client,
            Method::Put,
            "/texts/note",
            &token,
            Some(&json!({"text": "keep"}))
        )
        .0,
        Status::Ok
    );
    for body in [
        Value::Null,
        json!([]),
        json!("text"),
        json!({}),
        json!({"text": null}),
        json!({"text": true}),
        json!({"text": 42}),
        json!({"text": []}),
        json!({"text": {}}),
        json!({"text": "ok", "extra": true}),
    ] {
        for (method, path) in [(Method::Post, "/echo"), (Method::Put, "/texts/note")] {
            assert_eq!(
                request(&client, method, path, &token, Some(&body)).0,
                Status::BadRequest,
                "{path}: {body}"
            );
        }
    }
    for path in [
        "/texts/".to_owned(),
        "/texts/bad.name".into(),
        "/texts/a/b".into(),
        format!("/texts/{}", "a".repeat(65)),
    ] {
        for method in [Method::Put, Method::Get, Method::Delete] {
            let body = json!({"text": "hello"});
            assert_eq!(
                request(
                    &client,
                    method,
                    &path,
                    &token,
                    (method == Method::Put).then_some(&body)
                )
                .0,
                Status::BadRequest,
                "{method} {path}"
            );
        }
    }
    let paths = ["/texts/x".to_owned(), format!("/texts/{}", "a".repeat(64))];
    for path in &paths {
        let text = "😀".repeat(16_384);
        assert_eq!(
            request(
                &client,
                Method::Put,
                path,
                &token,
                Some(&json!({"text": text}))
            )
            .0,
            Status::Ok
        );
        assert_eq!(
            request(&client, Method::Get, path, &token, None),
            (Status::Ok, json!({"data": text}))
        );
        assert_eq!(
            request(&client, Method::Delete, path, &token, None).0,
            Status::Ok
        );
    }
    for text in ["a".repeat(65_537), format!("{}a", "😀".repeat(16_384))] {
        for (method, path) in [(Method::Post, "/echo"), (Method::Put, "/texts/note")] {
            assert_eq!(
                request(&client, method, path, &token, Some(&json!({"text": text}))).0,
                Status::PayloadTooLarge
            );
        }
    }
    assert_eq!(
        request(&client, Method::Get, "/texts/note", &token, None),
        (Status::Ok, json!({"data": "keep"}))
    );
}

#[test]
fn every_json_endpoint_rejects_malformed_and_oversized_wire_bodies() {
    let client = Client::tracked(create_app()).unwrap();
    let account = register(&client, "alice", "password1");
    let token = login(&client, &account);
    for (method, path, valid) in [
        (Method::Post, "/echo", json!({"text": "hello"})),
        (Method::Put, "/texts/note", json!({"text": "hello"})),
        (
            Method::Post,
            "/users",
            json!({"username": "bob", "password": "password1"}),
        ),
        (Method::Post, "/sessions", account),
    ] {
        for bytes in [
            Vec::new(),
            b"not JSON".to_vec(),
            b"{}{}".to_vec(),
            vec![0xff],
            br#"{"text":"\uD800"}"#.to_vec(),
            br#"{"text":"\uDC00"}"#.to_vec(),
        ] {
            assert_eq!(
                client
                    .req(method, path)
                    .header(ContentType::JSON)
                    .header(Header::new("Authorization", token.clone()))
                    .body(bytes)
                    .dispatch()
                    .status(),
                Status::BadRequest,
                "{path}"
            );
        }
        let wire = valid.to_string();
        let exact = format!("{wire}{}", " ".repeat(524_288 - wire.len()));
        assert_eq!(
            client
                .req(method, path)
                .header(ContentType::JSON)
                .header(Header::new("Authorization", token.clone()))
                .body(format!("{exact} "))
                .dispatch()
                .status(),
            Status::PayloadTooLarge
        );
        let expected = if path == "/users" {
            Status::Created
        } else {
            Status::Ok
        };
        assert_eq!(
            client
                .req(method, path)
                .header(ContentType::JSON)
                .header(Header::new("Authorization", token.clone()))
                .body(exact)
                .dispatch()
                .status(),
            expected
        );
        assert_eq!(
            request(&client, Method::Get, "/ping", "", None).0,
            Status::Ok
        );
    }
}

#[test]
fn lists_and_same_named_texts_are_isolated_and_case_sensitive() {
    let client = Client::tracked(create_app()).unwrap();
    let alice = login(&client, &register(&client, "alice", "password1"));
    let bob = login(&client, &register(&client, "bob", "password1"));
    for name in ["z", "note2", "a", "_", "note10", "A", "0", "-"] {
        assert_eq!(
            request(
                &client,
                Method::Put,
                &format!("/texts/{name}"),
                &alice,
                Some(&json!({"text": name}))
            )
            .0,
            Status::Ok
        );
    }
    assert_eq!(
        request(&client, Method::Get, "/texts", &alice, None),
        (
            Status::Ok,
            json!({"data": ["-", "0", "A", "_", "a", "note10", "note2", "z"]})
        )
    );
    assert_eq!(
        request(&client, Method::Get, "/texts", &bob, None),
        (Status::Ok, json!({"data": []}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/a", &bob, None).0,
        Status::NotFound
    );
    assert_eq!(
        request(&client, Method::Delete, "/texts/a", &bob, None).0,
        Status::NotFound
    );
    assert_eq!(
        request(
            &client,
            Method::Put,
            "/texts/a",
            &bob,
            Some(&json!({"text": "bob"}))
        )
        .0,
        Status::Ok
    );
    assert_eq!(
        request(
            &client,
            Method::Put,
            "/texts/a",
            &alice,
            Some(&json!({"text": "alice changed"}))
        )
        .0,
        Status::Ok
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/a", &bob, None),
        (Status::Ok, json!({"data": "bob"}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/a", &alice, None),
        (Status::Ok, json!({"data": "alice changed"}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/A", &alice, None),
        (Status::Ok, json!({"data": "A"}))
    );
    assert_eq!(
        request(&client, Method::Delete, "/texts/a", &alice, None).0,
        Status::Ok
    );
    assert_eq!(
        request(&client, Method::Get, "/texts", &alice, None),
        (
            Status::Ok,
            json!({"data": ["-", "0", "A", "_", "note10", "note2", "z"]})
        )
    );
    assert_eq!(
        request(&client, Method::Get, "/texts", &bob, None),
        (Status::Ok, json!({"data": ["a"]}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/a", &bob, None),
        (Status::Ok, json!({"data": "bob"}))
    );
    assert_eq!(
        request(&client, Method::Delete, "/users/me", &alice, None).0,
        Status::Ok
    );
    assert_eq!(
        request(&client, Method::Get, "/texts", &bob, None),
        (Status::Ok, json!({"data": ["a"]}))
    );
    assert_eq!(
        request(&client, Method::Delete, "/texts/a", &bob, None).0,
        Status::Ok
    );
    assert_eq!(
        request(&client, Method::Delete, "/texts/a", &bob, None).0,
        Status::NotFound
    );
    assert_eq!(
        request(&client, Method::Get, "/texts", &bob, None),
        (Status::Ok, json!({"data": []}))
    );
}

#[test]
fn known_paths_reject_unsupported_methods_and_unknown_paths_return_404() {
    let client = Client::tracked(create_app()).unwrap();
    let methods = [
        Method::Get,
        Method::Post,
        Method::Put,
        Method::Delete,
        Method::Patch,
        Method::Head,
        Method::Options,
        Method::Trace,
        Method::Connect,
    ];
    for (path, allowed) in [
        ("/ping", &[Method::Get][..]),
        ("/echo", &[Method::Post][..]),
        ("/users", &[Method::Post][..]),
        ("/sessions", &[Method::Post][..]),
        ("/sessions/current", &[Method::Delete][..]),
        ("/users/me", &[Method::Delete][..]),
        ("/texts", &[Method::Get][..]),
        (
            "/texts/note",
            &[Method::Get, Method::Put, Method::Delete][..],
        ),
    ] {
        for method in methods {
            if !allowed.contains(&method) {
                assert_eq!(
                    client.req(method, path).dispatch().status(),
                    Status::MethodNotAllowed,
                    "{method} {path}"
                );
            }
        }
    }
    for method in methods {
        assert_eq!(
            client.req(method, "/unknown").dispatch().status(),
            Status::NotFound
        );
    }
    assert_eq!(
        request(&client, Method::Get, "/ping", "", None).0,
        Status::Ok
    );
}
