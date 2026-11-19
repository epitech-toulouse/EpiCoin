//! Test harness: the real API router on a throw-away Postgres database, talking over HTTP
//! to a real in-process node that produces blocks every few milliseconds.
#![allow(dead_code)]

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use epicoin_api::config::Settings;
use epicoin_api::mailer::{Mail, MemoryMailer};
use epicoin_api::models::{Role, User};
use epicoin_api::state::AppState;
use epicoin_api::users::{self, NewUser};
use epicoin_api::vault::{MasterKey, Vault};
use epicoin_api::{authority, secrets};
use epicoin_core::{Address, Genesis, Keypair, DEFAULT_QUOTA_PERIOD_MS};
use epicoin_node::{now_ms, Node, NodeClient, NodeOptions};
use serde_json::Value;
use sqlx::PgPool;
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

pub const PASSWORD: &str = "correct horse battery staple";

// --- Log capture -----------------------------------------------------------------------

#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

static LOGS: OnceLock<LogBuffer> = OnceLock::new();

/// Captures every log line of the process (all crates, DEBUG and above) in memory.
fn capture_logs() -> &'static LogBuffer {
    LOGS.get_or_init(|| {
        let buffer = LogBuffer::default();
        let writer = buffer.clone();
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .try_init();
        buffer
    })
}

/// Everything logged so far by this test binary.
pub fn logs() -> String {
    String::from_utf8_lossy(&capture_logs().0.lock().unwrap()).into_owned()
}

// --- The application under test --------------------------------------------------------

pub struct TestApp {
    pub state: AppState,
    pub router: Router,
    pub db: PgPool,
    pub mailer: Arc<MemoryMailer>,
    pub node: Arc<Node>,
    pub node_url: String,
    pub genesis: Genesis,
    pub validator_key: Keypair,
    pub admin_key: Keypair,
    pub treasury_key: Keypair,
    pub master_key_hex: String,
    /// Every HTTP response produced during the test: status, headers and body.
    pub transcript: Arc<Mutex<Vec<String>>>,
    counter: AtomicUsize,
    _node_dir: TempDir,
}

pub struct Seeded {
    pub id: Uuid,
    pub email: String,
    pub role: Role,
    pub address: Address,
}

impl TestApp {
    pub async fn start(db: PgPool) -> Self {
        Self::start_with(db, Settings::development()).await
    }

    pub async fn start_with(db: PgPool, settings: Settings) -> Self {
        capture_logs();
        let validator_key = Keypair::generate();
        let admin_key = Keypair::generate();
        let treasury_key = Keypair::generate();
        let genesis = Genesis {
            chain_id: "epicoin-apitest".into(),
            timestamp: now_ms() - 1_000,
            validators: vec![validator_key.public_key()],
            admin: admin_key.address(),
            treasury: treasury_key.address(),
            quota_period_ms: DEFAULT_QUOTA_PERIOD_MS,
        };
        let node_dir = TempDir::new().unwrap();
        let node = Arc::new(
            Node::open(
                node_dir.path(),
                &genesis,
                Some(validator_key.clone()),
                NodeOptions::default(),
            )
            .unwrap(),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let node_url = format!("http://{}", listener.local_addr().unwrap());
        let served = node.clone();
        tokio::spawn(async move {
            axum::serve(listener, epicoin_node::http::router(served))
                .await
                .unwrap();
        });
        tokio::spawn(epicoin_node::producer::run(
            node.clone(),
            Duration::from_millis(15),
            None,
        ));

        let settings = Settings {
            upload_dir: node_dir.path().join("uploads"),
            ..settings
        };
        let master_key_hex = MasterKey::generate_hex().to_string();
        let mailer = Arc::new(MemoryMailer::default());
        let state = AppState::new(
            db.clone(),
            NodeClient::new(&node_url).unwrap(),
            Vault::new(MasterKey::from_hex(&master_key_hex).unwrap(), None),
            mailer.clone(),
            settings,
            admin_key.clone(),
            treasury_key.clone(),
        );
        Self {
            router: epicoin_api::router(state.clone()),
            state,
            db,
            mailer,
            node,
            node_url,
            genesis,
            validator_key,
            admin_key,
            treasury_key,
            master_key_hex,
            transcript: Arc::default(),
            counter: AtomicUsize::new(0),
            _node_dir: node_dir,
        }
    }

    /// The same application "restarted" with another vault (after a master-key rotation).
    pub fn restarted_with(self, vault: Vault) -> Self {
        let state = AppState::new(
            self.db.clone(),
            self.state.node.clone(),
            vault,
            self.mailer.clone(),
            self.state.settings.clone(),
            self.admin_key.clone(),
            self.treasury_key.clone(),
        );
        Self {
            router: epicoin_api::router(state.clone()),
            state,
            ..self
        }
    }

    /// A browser-like client without a session.
    pub fn client(&self) -> Client {
        Client {
            router: self.router.clone(),
            cookie: None,
            csrf: None,
            transcript: self.transcript.clone(),
        }
    }

    /// Creates an activated user directly in the database (bypassing the invitation
    /// email), with [`PASSWORD`]. Staff members and admins get their on-chain authority.
    pub async fn seed(&self, role: Role) -> Seeded {
        let n = self.counter.fetch_add(1, Ordering::SeqCst);
        let new = NewUser {
            email: format!("{}{n}@epitech.eu", role.as_str().replace('_', "")),
            first_name: format!("Prenom{n}"),
            last_name: format!("Nom{n}"),
            // Only students belong to a cohort.
            promo: if role == Role::Student { "pge1" } else { "" }.into(),
            role,
        };
        let mut conn = self.db.acquire().await.unwrap();
        let user = users::create(&mut conn, &self.state.vault, &new)
            .await
            .unwrap();
        sqlx::query("UPDATE users SET password_hash = $2, activated_at = now() WHERE id = $1")
            .bind(user.id)
            .bind(secrets::hash_password(PASSWORD).unwrap())
            .execute(&self.db)
            .await
            .unwrap();
        let address = epicoin_api::wallets::address_of(&self.db, user.id)
            .await
            .unwrap();
        if role.distributes() {
            authority::sync_user(&self.state, &user, address, None)
                .await
                .unwrap();
        }
        Seeded {
            id: user.id,
            email: user.email,
            role,
            address,
        }
    }

    /// Seeds a user of `role` and returns a client logged in as them.
    pub async fn login_as(&self, role: Role) -> (Client, Seeded) {
        let user = self.seed(role).await;
        let mut client = self.client();
        assert_eq!(client.login(&user.email, PASSWORD).await, StatusCode::OK);
        (client, user)
    }

    pub async fn user(&self, id: Uuid) -> Option<User> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.db)
            .await
            .unwrap()
    }

    /// Waits for an email to `address` (invitations are sent in the background).
    pub async fn mail_to(&self, address: &str) -> Mail {
        for _ in 0..200 {
            if let Some(mail) = self.mailer.last_to(address) {
                return mail;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no email was sent to {address}");
    }

    /// On-chain balance of an address, in units.
    pub fn balance(&self, address: &Address) -> u64 {
        self.node.account(address).balance
    }

    /// Polls `condition` for a few seconds (for effects that land in the background).
    pub async fn eventually(&self, what: &str, mut condition: impl FnMut() -> bool) {
        for _ in 0..300 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for: {what}");
    }

    pub fn transcript(&self) -> String {
        self.transcript.lock().unwrap().join("\n")
    }
}

/// Extracts the `token=…` of the link contained in an email.
pub fn token_in(mail: &Mail) -> String {
    let start = mail.body.find("token=").expect("the email contains a link") + "token=".len();
    mail.body[start..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect()
}

// --- HTTP client -----------------------------------------------------------------------

/// Behaves like a browser: keeps the session cookie, and sends the CSRF token it was given
/// with every state-changing request.
pub struct Client {
    router: Router,
    pub cookie: Option<String>,
    pub csrf: Option<String>,
    transcript: Arc<Mutex<Vec<String>>>,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Value,
}

impl Reply {
    pub fn error(&self) -> &str {
        self.body["error"].as_str().unwrap_or("")
    }
}

impl Client {
    pub async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        extra_headers: &[(&str, &str)],
    ) -> Reply {
        let mut request = Request::builder().method(method.clone()).uri(path);
        if let Some(cookie) = &self.cookie {
            request = request.header(header::COOKIE, format!("epicoin_session={cookie}"));
        }
        if !method.is_safe() {
            if let Some(csrf) = &self.csrf {
                request = request.header("x-csrf-token", csrf);
            }
        }
        for (name, value) in extra_headers {
            request = request.header(*name, *value);
        }
        let request = match body {
            Some(json) => request
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string())),
            None => request.body(Body::empty()),
        }
        .unwrap();

        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        self.transcript
            .lock()
            .unwrap()
            .push(format!("{method} {path} -> {status}\n{headers:?}\n{text}"));

        if let Some(set_cookie) = headers
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
        {
            let value = set_cookie
                .split(';')
                .next()
                .and_then(|pair| pair.split_once('='))
                .map(|(_, value)| value.to_owned())
                .unwrap_or_default();
            self.cookie = (!value.is_empty()).then_some(value);
        }
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if let Some(csrf) = body.get("csrf_token").and_then(Value::as_str) {
            self.csrf = Some(csrf.to_owned());
        }
        Reply {
            status,
            headers,
            body,
        }
    }

    pub async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, None, &[]).await
    }

    pub async fn post(&mut self, path: &str, body: Value) -> Reply {
        self.send(Method::POST, path, Some(body), &[]).await
    }

    pub async fn patch(&mut self, path: &str, body: Value) -> Reply {
        self.send(Method::PATCH, path, Some(body), &[]).await
    }

    pub async fn delete(&mut self, path: &str) -> Reply {
        self.send(Method::DELETE, path, None, &[]).await
    }

    pub async fn login(&mut self, email: &str, password: &str) -> StatusCode {
        self.post(
            "/api/auth/login",
            serde_json::json!({ "email": email, "password": password }),
        )
        .await
        .status
    }
}
