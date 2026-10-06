pub mod http;

use pbkdf2::pbkdf2_hmac;
use rand::{RngCore, rngs::OsRng};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;

pub const ROUTES: &[(&str, &str)] = &[
    ("GET", "/ping"),
    ("POST", "/echo"),
    ("POST", "/users"),
    ("POST", "/sessions"),
    ("DELETE", "/sessions/current"),
    ("DELETE", "/users/me"),
    ("GET", "/texts"),
    ("PUT", "/texts/{name}"),
    ("GET", "/texts/{name}"),
    ("DELETE", "/texts/{name}"),
];

pub fn route_error(method: &str, path: &str) -> Option<u16> {
    if path.starts_with("/texts/") {
        return (!matches!(method, "PUT" | "GET" | "DELETE")).then_some(405);
    }
    match ROUTES.iter().find(|(_, route)| *route == path) {
        None => Some(404),
        Some((allowed, _)) if *allowed != method => Some(405),
        Some(_) => None,
    }
}

pub struct User {
    pub salt: [u8; 16],
    pub digest: [u8; 32],
    pub token: Option<String>,
    pub token_issued_at: Option<Instant>,
    pub texts: BTreeMap<String, String>,
}

pub struct Service {
    pub users: Mutex<BTreeMap<String, User>>,
    token_ttl_seconds: NonZeroU64,
}

impl Default for Service {
    fn default() -> Self {
        Self::new(NonZeroU64::new(300).unwrap())
    }
}

impl User {
    fn accepts_token(&self, token: &str, now: Instant, ttl: Duration) -> bool {
        !token.is_empty()
            && self.token.as_deref() == Some(token)
            && self
                .token_issued_at
                .is_some_and(|issued| now.duration_since(issued) < ttl)
    }
}

pub fn error(status: u16, message: &str) -> (u16, Value) {
    (status, json!({"message": message}))
}

pub fn valid_name(name: &str, max: usize) -> bool {
    !name.is_empty()
        && name.len() <= max
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn password_hash(password: &str, salt: &[u8; 16]) -> [u8; 32] {
    let mut output = [0; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), salt, 100_000, &mut output);
    output
}

fn new_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Service {
    pub fn new(token_ttl_seconds: NonZeroU64) -> Self {
        Self {
            users: Mutex::new(BTreeMap::new()),
            token_ttl_seconds,
        }
    }

    fn finish_login(
        &self,
        name: &str,
        salt: [u8; 16],
        expected: [u8; 32],
        digest: [u8; 32],
    ) -> (u16, Value) {
        let mut users = self.users.lock().unwrap();
        let Some(user) = users.get_mut(name) else {
            return error(401, "Invalid username or password");
        };
        if user.salt != salt || !bool::from(digest.ct_eq(&expected)) {
            return error(401, "Invalid username or password");
        }
        let token = new_token();
        user.token = Some(token.clone());
        user.token_issued_at = Some(Instant::now());
        (
            200,
            json!({"data": {"token": token, "expires_in": self.token_ttl_seconds.get()}}),
        )
    }

    pub fn handle(
        &self,
        method: &str,
        path: &str,
        body: &Value,
        authorization: &str,
    ) -> (u16, Value) {
        if let Some(status) = route_error(method, path) {
            return error(
                status,
                if status == 404 {
                    "Not found"
                } else {
                    "Method not allowed"
                },
            );
        }
        if method == "GET" && path == "/ping" {
            return (200, json!({"data": "pong"}));
        }
        if method == "POST" && path == "/echo" {
            let Some(text) = body.get("text").and_then(Value::as_str) else {
                return error(400, "Expected text string");
            };
            if body.as_object().map(|fields| fields.len()) != Some(1) {
                return error(400, "Expected only text field");
            }
            // String length counts UTF-8 bytes, as required by the text limit.
            if text.len() > 65_536 {
                return error(413, "Text too large");
            }
            return (200, json!({"data": text}));
        }
        if matches!(method, "GET" | "DELETE") && path.starts_with("/texts/") {
            let name = &path["/texts/".len()..];
            if !valid_name(name, 64) {
                return error(400, "Invalid text name");
            }
        }
        if method == "PUT" && path.starts_with("/texts/") {
            let name = &path["/texts/".len()..];
            let Some(text) = body.get("text").and_then(Value::as_str) else {
                return error(400, "Expected text string");
            };
            if !valid_name(name, 64) || body.as_object().map(|fields| fields.len()) != Some(1) {
                return error(400, "Invalid text fields");
            }
            if text.len() > 65_536 {
                return error(413, "Text too large");
            }
        }
        if method == "POST" && matches!(path, "/users" | "/sessions") {
            let Some(name) = body.get("username").and_then(Value::as_str) else {
                return error(400, "Expected username");
            };
            let Some(password) = body.get("password").and_then(Value::as_str) else {
                return error(400, "Expected password");
            };
            if body.as_object().map(|v| v.len()) != Some(2)
                || !valid_name(name, 32)
                || !(8..=128).contains(&password.chars().count())
            {
                return error(400, "Invalid account fields");
            }
            if path == "/users" {
                let mut salt = [0; 16];
                OsRng.fill_bytes(&mut salt);
                let digest = password_hash(password, &salt);
                let mut users = self.users.lock().unwrap();
                if users.contains_key(name) {
                    return error(409, "Username exists");
                }
                users.insert(
                    name.into(),
                    User {
                        salt,
                        digest,
                        token: None,
                        token_issued_at: None,
                        texts: BTreeMap::new(),
                    },
                );
                return (201, json!({"data": {"username": name}}));
            }
            let (salt, expected) = {
                let users = self.users.lock().unwrap();
                let Some(user) = users.get(name) else {
                    return error(401, "Invalid username or password");
                };
                (user.salt, user.digest)
            };
            let digest = password_hash(password, &salt);
            return self.finish_login(name, salt, expected, digest);
        }
        let protected = matches!(path, "/texts" | "/sessions/current" | "/users/me")
            || matches!(method, "PUT" | "GET" | "DELETE") && path.starts_with("/texts/");
        if protected {
            let token = authorization.strip_prefix("Bearer ").unwrap_or("");
            let mut users = self.users.lock().unwrap();
            let now = Instant::now();
            let ttl = Duration::from_secs(self.token_ttl_seconds.get());
            let name = users
                .iter()
                .find(|(_, user)| user.accepts_token(token, now, ttl))
                .map(|(name, _)| name.clone());
            let Some(name) = name else {
                return error(401, "Login required");
            };
            if method == "DELETE" && path == "/users/me" {
                users.remove(&name);
                return (200, json!({"data": null}));
            }
            let user = users.get_mut(&name).unwrap();
            if method == "DELETE" && path == "/sessions/current" {
                user.token = None;
                user.token_issued_at = None;
                return (200, json!({"data": null}));
            }
            if method == "GET" && path == "/texts" {
                return (200, json!({"data": user.texts.keys().collect::<Vec<_>>()}));
            }
            if method == "PUT" && path.starts_with("/texts/") {
                let name = &path["/texts/".len()..];
                let text = body["text"].as_str().unwrap();
                user.texts.insert(name.to_owned(), text.to_owned());
                return (200, json!({"data": null}));
            }
            if method == "GET" && path.starts_with("/texts/") {
                let name = &path["/texts/".len()..];
                let Some(text) = user.texts.get(name) else {
                    return error(404, "Text not found");
                };
                return (200, json!({"data": text}));
            }
            if method == "DELETE" && path.starts_with("/texts/") {
                let name = &path["/texts/".len()..];
                if user.texts.remove(name).is_none() {
                    return error(404, "Text not found");
                }
                return (200, json!({"data": null}));
            }
        }
        error(404, "Not found")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_is_invalid_at_the_exact_expiry_boundary() {
        let issued = Instant::now();
        let user = User {
            salt: [0; 16],
            digest: [0; 32],
            token: Some("sample".into()),
            token_issued_at: Some(issued),
            texts: BTreeMap::new(),
        };
        let ttl = Duration::from_secs(1);
        assert!(user.accepts_token("sample", issued, ttl));
        assert!(user.accepts_token("sample", issued + ttl - Duration::from_nanos(1), ttl));
        assert!(!user.accepts_token("sample", issued + ttl, ttl));
        assert!(!user.accepts_token("sample", issued + ttl + Duration::from_nanos(1), ttl));
    }

    #[test]
    fn pending_old_login_cannot_change_reregistered_account() {
        let service = Service::default();
        let account = json!({"username": "alice", "password": "password1"});
        assert_eq!(service.handle("POST", "/users", &account, "").0, 201);
        let login = service.handle("POST", "/sessions", &account, "").1;
        let token = format!("Bearer {}", login["data"]["token"].as_str().unwrap());
        // Capture the first phase of an old account's pending login.
        let (salt, expected) = {
            let users = service.users.lock().unwrap();
            (users["alice"].salt, users["alice"].digest)
        };
        let digest = password_hash("password1", &salt);
        assert_eq!(
            service
                .handle("DELETE", "/users/me", &Value::Null, &token)
                .0,
            200
        );
        assert_eq!(service.finish_login("alice", salt, expected, digest).0, 401);
        assert_eq!(service.handle("POST", "/users", &account, "").0, 201);
        let login = service.handle("POST", "/sessions", &account, "");
        assert_eq!(login.0, 200);
        let new_token = login.1["data"]["token"].as_str().unwrap();
        assert_eq!(service.finish_login("alice", salt, expected, digest).0, 401);
        assert_eq!(
            service.users.lock().unwrap()["alice"].token.as_deref(),
            Some(new_token)
        );
    }
    #[test]
    fn account_lifecycle() {
        let service = Service::default();
        let account = json!({"username":"alice", "password":"password1"});
        assert_eq!(service.handle("POST", "/users", &account, "").0, 201);
        assert_eq!(service.handle("POST", "/users", &account, "").0, 409);
        let login = service.handle("POST", "/sessions", &account, "").1;
        let old = format!("Bearer {}", login["data"]["token"].as_str().unwrap());
        let login = service.handle("POST", "/sessions", &account, "").1;
        let current = format!("Bearer {}", login["data"]["token"].as_str().unwrap());
        assert_ne!(old, current);
        assert_eq!(service.handle("GET", "/texts", &Value::Null, &old).0, 401);
        assert_eq!(
            service.handle("GET", "/texts", &Value::Null, &current),
            (200, json!({"data":[]}))
        );
        assert_eq!(
            service
                .handle("DELETE", "/sessions/current", &Value::Null, &current)
                .0,
            200
        );
        assert_eq!(
            service.handle("GET", "/texts", &Value::Null, &current).0,
            401
        );
    }
}
