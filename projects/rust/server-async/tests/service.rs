use rm_server_async::Service;
use serde_json::{Value, json};

fn register_and_login(service: &Service, username: &str) -> String {
    let account = json!({"username": username, "password": "password1"});
    assert_eq!(service.handle("POST", "/users", &account, "").0, 201);
    let response = service.handle("POST", "/sessions", &account, "").1;
    format!("Bearer {}", response["data"]["token"].as_str().unwrap())
}

#[test]
fn delete_account_cleans_data_and_revokes_old_identity() {
    let service = Service::default();
    let alice = register_and_login(&service, "alice");
    let bob = register_and_login(&service, "bob");
    for token in [&alice, &bob] {
        assert_eq!(
            service
                .handle("PUT", "/texts/note", &json!({"text": "hello"}), token)
                .0,
            200
        );
    }
    for token in ["", "Bearer invalid"] {
        assert_eq!(
            service.handle("DELETE", "/users/me", &Value::Null, token).0,
            401
        );
    }
    assert_eq!(
        service.handle("DELETE", "/users/me", &Value::Null, &alice),
        (200, json!({"data": null}))
    );
    assert!(!service.users.lock().unwrap().contains_key("alice"));
    assert_eq!(
        service
            .handle("DELETE", "/users/me", &Value::Null, &alice)
            .0,
        401
    );
    let account = json!({"username": "alice", "password": "password1"});
    assert_eq!(service.handle("POST", "/sessions", &account, "").0, 401);
    let new_alice = register_and_login(&service, "alice");
    for (method, path, body) in [
        ("GET", "/texts", Value::Null),
        ("GET", "/texts/note", Value::Null),
        ("PUT", "/texts/note", json!({"text": "old write"})),
        ("DELETE", "/texts/note", Value::Null),
        ("DELETE", "/sessions/current", Value::Null),
        ("DELETE", "/users/me", Value::Null),
    ] {
        assert_eq!(service.handle(method, path, &body, &alice).0, 401);
    }
    assert_eq!(
        service.handle("GET", "/texts", &Value::Null, &new_alice),
        (200, json!({"data": []}))
    );
    assert_eq!(
        service.handle("GET", "/texts/note", &Value::Null, &bob),
        (200, json!({"data": "hello"}))
    );
}

#[test]
fn account_deletion_and_text_write_remain_consistent() {
    let service = std::sync::Arc::new(Service::default());
    let token = register_and_login(&service, "alice");
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let worker = {
        let service = service.clone();
        let token = token.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            service
                .handle("PUT", "/texts/note", &json!({"text": "race"}), &token)
                .0
        })
    };
    barrier.wait();
    assert_eq!(
        service
            .handle("DELETE", "/users/me", &Value::Null, &token)
            .0,
        200
    );
    assert!(matches!(worker.join().unwrap(), 200 | 401));
    assert!(!service.users.lock().unwrap().contains_key("alice"));
    assert_eq!(
        service
            .handle("PUT", "/texts/note", &json!({"text": "late"}), &token)
            .0,
        401
    );
}

#[test]
fn delete_text_updates_list_and_preserves_other_users() {
    let service = Service::default();
    let alice = register_and_login(&service, "alice");
    let bob = register_and_login(&service, "bob");
    for (token, text) in [(&alice, ""), (&bob, "bob text")] {
        assert_eq!(
            service
                .handle("PUT", "/texts/note", &json!({"text": text}), token)
                .0,
            200
        );
    }
    assert_eq!(
        service
            .handle("PUT", "/texts/z", &json!({"text": "keep"}), &alice)
            .0,
        200
    );
    assert_eq!(
        service.handle("GET", "/texts", &Value::Null, &alice),
        (200, json!({"data": ["note", "z"]}))
    );
    assert_eq!(
        service.handle("DELETE", "/texts/note", &Value::Null, &alice),
        (200, json!({"data": null}))
    );
    assert_eq!(
        service.handle("GET", "/texts/note", &Value::Null, &alice).0,
        404
    );
    assert_eq!(
        service
            .handle("DELETE", "/texts/note", &Value::Null, &alice)
            .0,
        404
    );
    assert_eq!(
        service.handle("GET", "/texts", &Value::Null, &alice),
        (200, json!({"data": ["z"]}))
    );
    assert_eq!(
        service.handle("GET", "/texts/note", &Value::Null, &bob),
        (200, json!({"data": "bob text"}))
    );
    for token in ["", "Bearer invalid"] {
        assert_eq!(
            service.handle("DELETE", "/texts/z", &Value::Null, token).0,
            401
        );
    }
    for path in [
        "/texts/",
        "/texts/a/b",
        "/texts/bad.name",
        "/texts/你好",
        &format!("/texts/{}", "a".repeat(65)),
    ] {
        assert_eq!(service.handle("DELETE", path, &Value::Null, &alice).0, 400);
    }
    assert_eq!(
        service.handle("GET", "/texts/z", &Value::Null, &alice),
        (200, json!({"data": "keep"}))
    );
    assert_eq!(
        service.handle("DELETE", "/texts/z", &Value::Null, &alice).0,
        200
    );
    assert_eq!(
        service.handle("GET", "/texts", &Value::Null, &alice),
        (200, json!({"data": []}))
    );
}

#[test]
fn concurrent_deletion_has_one_success() {
    let service = std::sync::Arc::new(Service::default());
    let token = register_and_login(&service, "alice");
    assert_eq!(
        service
            .handle("PUT", "/texts/note", &json!({"text": "hello"}), &token)
            .0,
        200
    );
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let service = service.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                service
                    .handle("DELETE", "/texts/note", &Value::Null, &token)
                    .0
            })
        })
        .collect();
    let statuses: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(statuses.iter().filter(|&&status| status == 200).count(), 1);
    assert_eq!(statuses.iter().filter(|&&status| status == 404).count(), 3);
}

#[test]
fn put_text_creates_and_overwrites_for_current_user() {
    let service = Service::default();
    let authorization = register_and_login(&service, "alice");

    for text in ["", "你好\nRM\n", &"😀".repeat(16_384)] {
        assert_eq!(
            service.handle(
                "PUT",
                "/texts/note-1",
                &json!({"text": text}),
                &authorization,
            ),
            (200, json!({"data": null}))
        );
        assert_eq!(service.users.lock().unwrap()["alice"].texts["note-1"], text);
        assert_eq!(
            service.handle("GET", "/texts/note-1", &Value::Null, &authorization),
            (200, json!({"data": text}))
        );
    }

    assert_eq!(
        service.handle("GET", "/texts", &Value::Null, &authorization),
        (200, json!({"data": ["note-1"]}))
    );
}

#[test]
fn get_text_validates_identity_name_and_user_isolation() {
    let service = Service::default();
    let alice = register_and_login(&service, "alice");
    let bob = register_and_login(&service, "bob");
    let path = format!("/texts/{}", "a".repeat(64));
    assert_eq!(
        service
            .handle("PUT", &path, &json!({"text": "alice text"}), &alice)
            .0,
        200
    );
    assert_eq!(service.handle("GET", &path, &Value::Null, &bob).0, 404);
    assert_eq!(
        service
            .handle("PUT", &path, &json!({"text": "bob text"}), &bob)
            .0,
        200
    );
    for (token, expected) in [(&alice, "alice text"), (&bob, "bob text")] {
        assert_eq!(
            service.handle("GET", &path, &Value::Null, token),
            (200, json!({"data": expected}))
        );
    }
    for token in ["", "Bearer invalid", "invalid"] {
        assert_eq!(service.handle("GET", &path, &Value::Null, token).0, 401);
    }
    for path in [
        "/texts/",
        "/texts/a/b",
        "/texts/bad.name",
        "/texts/你好",
        &format!("/texts/{}", "a".repeat(65)),
    ] {
        assert_eq!(
            service.handle("GET", path, &Value::Null, &alice).0,
            400,
            "{path}"
        );
    }
    assert_eq!(
        service
            .handle("GET", "/texts/missing", &Value::Null, &alice)
            .0,
        404
    );
    assert_eq!(
        service.handle("GET", "/texts/A", &Value::Null, &alice).0,
        404
    );
    assert_eq!(
        service
            .handle("DELETE", "/sessions/current", &Value::Null, &alice)
            .0,
        200
    );
    assert_eq!(service.handle("GET", &path, &Value::Null, &alice).0, 401);
}

#[test]
fn put_text_validates_auth_name_body_and_size() {
    let service = Service::default();
    let authorization = register_and_login(&service, "alice");
    assert_eq!(
        service
            .handle("PUT", "/texts/note", &json!({"text": "ok"}), "")
            .0,
        401
    );
    for path in [
        "/texts/",
        "/texts/a/b",
        "/texts/你好",
        &format!("/texts/{}", "a".repeat(65)),
    ] {
        assert_eq!(
            service
                .handle("PUT", path, &json!({"text": "ok"}), &authorization)
                .0,
            400,
            "{path}"
        );
    }
    for body in [
        Value::Null,
        json!({}),
        json!({"text": 1}),
        json!({"text": "ok", "extra": true}),
    ] {
        assert_eq!(
            service
                .handle("PUT", "/texts/note", &body, &authorization)
                .0,
            400,
            "{body}"
        );
    }
    assert_eq!(
        service
            .handle(
                "PUT",
                "/texts/note",
                &json!({"text": "a".repeat(65_537)}),
                &authorization,
            )
            .0,
        413
    );
}

#[test]
fn echo_preserves_text_and_enforces_byte_limit() {
    let service = Service::default();
    for text in [
        "".to_owned(),
        " 你好\nRM\n ".to_owned(),
        "😀".repeat(16_384),
    ] {
        assert_eq!(
            service.handle("POST", "/echo", &json!({"text": text}), ""),
            (200, json!({"data": text}))
        );
    }
    for text in ["a".repeat(65_537), format!("{}a", "😀".repeat(16_384))] {
        assert_eq!(
            service
                .handle("POST", "/echo", &json!({"text": text}), "")
                .0,
            413
        );
    }
    assert!(service.users.lock().unwrap().is_empty());
}

#[test]
fn echo_requires_exactly_one_string_field() {
    let service = Service::default();
    for body in [
        Value::Null,
        json!([]),
        json!("text"),
        json!({}),
        json!({"text": null}),
        json!({"text": 42}),
        json!({"text": true}),
        json!({"text": []}),
        json!({"text": {}}),
        json!({"text": "hello", "extra": "field"}),
    ] {
        assert_eq!(service.handle("POST", "/echo", &body, "").0, 400, "{body}");
    }
}

#[test]
fn input_validation_and_baseline() {
    let service = Service::default();
    assert_eq!(
        service.handle("GET", "/ping", &Value::Null, ""),
        (200, json!({"data":"pong"}))
    );
    for body in [
        Value::Null,
        json!([]),
        json!({"username":true,"password":"password1"}),
        json!({"username":"a/b","password":"password1"}),
    ] {
        assert_eq!(service.handle("POST", "/users", &body, "").0, 400);
    }
    assert_eq!(service.handle("GET", "/texts", &Value::Null, "").0, 401);
    assert_eq!(service.handle("GET", "/missing", &Value::Null, "").0, 404);
}

#[test]
fn concurrent_registration_has_one_winner() {
    let service = std::sync::Arc::new(Service::default());
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let service = service.clone();
            std::thread::spawn(move || {
                service
                    .handle(
                        "POST",
                        "/users",
                        &json!({"username":"alice","password":"password1"}),
                        "",
                    )
                    .0
            })
        })
        .collect();
    let statuses: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(statuses.iter().filter(|&&s| s == 201).count(), 1);
    assert_eq!(statuses.iter().filter(|&&s| s == 409).count(), 3);
}
