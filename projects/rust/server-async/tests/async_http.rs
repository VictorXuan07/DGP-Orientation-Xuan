use rm_server_async::http::create_app;
use rocket::http::{ContentType, Header, Method, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};

async fn request(
    client: &Client,
    method: Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (Status, Value) {
    let mut request = client.req(method, path);
    if !token.is_empty() {
        request = request.header(Header::new("Authorization", format!("Bearer {token}")));
    }
    if let Some(body) = body {
        request = request.header(ContentType::JSON).body(body.to_string());
    }
    let response = request.dispatch().await;
    let status = response.status();
    (status, response.into_json::<Value>().await.unwrap())
}

async fn register_and_login(client: &Client, name: &str) -> String {
    let account = json!({"username": name, "password": "password1"});
    assert_eq!(
        request(client, Method::Post, "/users", "", Some(account.clone()))
            .await
            .0,
        Status::Created
    );
    let (status, login) = request(client, Method::Post, "/sessions", "", Some(account)).await;
    assert_eq!(status, Status::Ok);
    login["data"]["token"].as_str().unwrap().to_owned()
}

#[rocket::async_test]
async fn concurrent_registration_and_invalid_request_preserve_service() {
    let client = Client::tracked(create_app()).await.unwrap();
    let account = json!({"username": "alice", "password": "password1"}).to_string();
    let first = client
        .post("/users")
        .header(ContentType::JSON)
        .body(&account);
    let second = client
        .post("/users")
        .header(ContentType::JSON)
        .body(&account);
    let invalid = client.post("/users").header(ContentType::JSON).body("{x}");
    let ping = client.get("/ping");

    let (first, second, invalid, ping) = rocket::tokio::join!(
        first.dispatch(),
        second.dispatch(),
        invalid.dispatch(),
        ping.dispatch(),
    );
    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|&&status| status == Status::Created)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|&&status| status == Status::Conflict)
            .count(),
        1
    );
    let created = if first.status() == Status::Created {
        first
    } else {
        second
    };
    assert_eq!(
        created.into_json::<Value>().await.unwrap(),
        json!({"data": {"username": "alice"}})
    );
    assert_eq!(invalid.status(), Status::BadRequest);
    assert_eq!(ping.status(), Status::Ok);
    assert_eq!(
        ping.into_json::<Value>().await.unwrap(),
        json!({"data": "pong"})
    );

    let login = client
        .post("/sessions")
        .header(ContentType::JSON)
        .body(account)
        .dispatch()
        .await;
    assert_eq!(login.status(), Status::Ok);
    let login = login.into_json::<Value>().await.unwrap();
    let token = login["data"]["token"].as_str().unwrap();
    assert!(!token.is_empty());
    let list = client
        .get("/texts")
        .header(Header::new("Authorization", format!("Bearer {token}")))
        .dispatch()
        .await;
    assert_eq!(list.status(), Status::Ok);
    assert_eq!(
        list.into_json::<Value>().await.unwrap(),
        json!({"data": []})
    );
}

#[rocket::async_test]
async fn text_operations_and_login_replacement_remain_consistent() {
    for method in [Method::Get, Method::Put, Method::Delete] {
        let client = Client::tracked(create_app()).await.unwrap();
        let old = register_and_login(&client, "alice").await;
        let bob = register_and_login(&client, "bob").await;
        for token in [&old, &bob] {
            assert_eq!(
                request(
                    &client,
                    Method::Put,
                    "/texts/note",
                    token,
                    Some(json!({"text": "before"}))
                )
                .await,
                (Status::Ok, json!({"data": null}))
            );
        }

        let body = (method == Method::Put).then(|| json!({"text": "after"}));
        let (operation, login) = rocket::tokio::join!(
            request(&client, method, "/texts/note", &old, body),
            request(
                &client,
                Method::Post,
                "/sessions",
                "",
                Some(json!({"username": "alice", "password": "password1"}))
            ),
        );
        assert_eq!(login.0, Status::Ok);
        let next = login.1["data"]["token"].as_str().unwrap();
        assert_ne!(next, old);
        assert!(
            operation.0 == Status::Ok || operation.0 == Status::Unauthorized,
            "{method}: {operation:?}"
        );
        if operation.0 == Status::Ok {
            assert_eq!(
                operation.1,
                if method == Method::Get {
                    json!({"data": "before"})
                } else {
                    json!({"data": null})
                }
            );
        }

        let expected = match (method, operation.0 == Status::Ok) {
            (Method::Delete, true) => None,
            (Method::Put, true) => Some("after"),
            _ => Some("before"),
        };
        let read = request(&client, Method::Get, "/texts/note", next, None).await;
        if let Some(text) = expected {
            assert_eq!(read, (Status::Ok, json!({"data": text})));
        } else {
            assert_eq!(read.0, Status::NotFound);
        }
        assert_eq!(
            request(&client, Method::Get, "/texts", next, None).await,
            (
                Status::Ok,
                if expected.is_some() {
                    json!({"data": ["note"]})
                } else {
                    json!({"data": []})
                }
            )
        );

        for method in [Method::Get, Method::Put, Method::Delete] {
            let body = (method == Method::Put).then(|| json!({"text": "late"}));
            assert_eq!(
                request(&client, method, "/texts/note", &old, body).await.0,
                Status::Unauthorized
            );
        }
        assert_eq!(
            request(&client, Method::Get, "/texts/note", next, None).await,
            read
        );
        assert_eq!(
            request(&client, Method::Get, "/texts/note", &bob, None).await,
            (Status::Ok, json!({"data": "before"}))
        );
        assert_eq!(
            request(&client, Method::Get, "/texts", &bob, None).await,
            (Status::Ok, json!({"data": ["note"]}))
        );
        assert_eq!(
            request(&client, Method::Get, "/ping", "", None).await,
            (Status::Ok, json!({"data": "pong"}))
        );
    }
}

#[rocket::async_test]
async fn text_write_and_account_deletion_do_not_restore_old_identity() {
    let client = Client::tracked(create_app()).await.unwrap();
    let old = register_and_login(&client, "alice").await;
    let bob = register_and_login(&client, "bob").await;
    for (token, text) in [(&old, "alice text"), (&bob, "bob text")] {
        assert_eq!(
            request(
                &client,
                Method::Put,
                "/texts/note",
                token,
                Some(json!({"text": text}))
            )
            .await,
            (Status::Ok, json!({"data": null}))
        );
    }

    let (write, deletion) = rocket::tokio::join!(
        request(
            &client,
            Method::Put,
            "/texts/note",
            &old,
            Some(json!({"text": "race"}))
        ),
        request(&client, Method::Delete, "/users/me", &old, None),
    );
    assert_eq!(deletion, (Status::Ok, json!({"data": null})));
    assert!(
        write.0 == Status::Ok || write.0 == Status::Unauthorized,
        "{write:?}"
    );
    if write.0 == Status::Ok {
        assert_eq!(write.1, json!({"data": null}));
    }
    let account = json!({"username": "alice", "password": "password1"});
    assert_eq!(
        request(&client, Method::Post, "/sessions", "", Some(account))
            .await
            .0,
        Status::Unauthorized
    );
    assert_eq!(
        request(
            &client,
            Method::Put,
            "/texts/note",
            &old,
            Some(json!({"text": "late"}))
        )
        .await
        .0,
        Status::Unauthorized
    );

    let next = register_and_login(&client, "alice").await;
    assert_ne!(next, old);
    for (method, path, body) in [
        (Method::Get, "/texts", None),
        (Method::Get, "/texts/note", None),
        (
            Method::Put,
            "/texts/note",
            Some(json!({"text": "old identity"})),
        ),
        (Method::Delete, "/texts/note", None),
        (Method::Delete, "/sessions/current", None),
        (Method::Delete, "/users/me", None),
    ] {
        assert_eq!(
            request(&client, method, path, &old, body).await.0,
            Status::Unauthorized,
            "{method} {path}"
        );
    }
    assert_eq!(
        request(&client, Method::Get, "/texts", &next, None).await,
        (Status::Ok, json!({"data": []}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/note", &next, None)
            .await
            .0,
        Status::NotFound
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/note", &bob, None).await,
        (Status::Ok, json!({"data": "bob text"}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts", &bob, None).await,
        (Status::Ok, json!({"data": ["note"]}))
    );

    assert_eq!(
        request(
            &client,
            Method::Put,
            "/texts/note",
            &next,
            Some(json!({"text": "new account"}))
        )
        .await,
        (Status::Ok, json!({"data": null}))
    );
    assert_eq!(
        request(&client, Method::Get, "/texts/note", &next, None).await,
        (Status::Ok, json!({"data": "new account"}))
    );
    assert_eq!(
        request(&client, Method::Get, "/ping", "", None).await,
        (Status::Ok, json!({"data": "pong"}))
    );
}
