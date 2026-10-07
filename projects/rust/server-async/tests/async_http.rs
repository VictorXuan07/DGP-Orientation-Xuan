use rm_server_async::http::create_app;
use rocket::http::{ContentType, Header, Status};
use rocket::local::asynchronous::Client;
use serde_json::{Value, json};

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
