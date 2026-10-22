//! Local server mode (`pdfi serve`): the web UI and a WebSocket carrying the
//! pdfi-api protocol, bound to the loopback interface only.
//!
//! Security (docs/DESIGN.md §4.9):
//! - listens on `127.0.0.1` only;
//! - a random token per start, given to the UI in the URL fragment and
//!   presented as a WebSocket subprotocol (`pdfi.token.<hex>`);
//! - `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (DNS rebinding);
//! - WebSocket `Origin` must be the server itself (or an explicitly added
//!   development origin);
//! - files are opened only below the allowed roots, after canonicalization.

mod fs;

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Request as HttpRequest, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::get;
use pdfi_api::{
    ApiError, ClientMessage, ErrorCode, Executor, OpenSource, Request, Response, ServerMessage,
    Session, Task,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

pub use fs::FsProvider;

/// WebSocket subprotocol spoken by the UI.
pub const SUBPROTOCOL: &str = "pdfi.v1";
const TOKEN_PREFIX: &str = "pdfi.token.";

#[derive(Debug, Clone, Default)]
pub struct ServeOptions {
    /// Files to open at startup; each is also allowed to be opened.
    pub files: Vec<PathBuf>,
    /// Directories whose contents may be browsed and opened.
    pub roots: Vec<PathBuf>,
    /// 0 picks a free port.
    pub port: u16,
    /// Additional allowed WebSocket origins (e.g. the Vite dev server).
    pub dev_origins: Vec<String>,
}

#[derive(rust_embed::Embed)]
#[folder = "../../apps/web/dist"]
#[allow_missing = true]
struct Assets;

/// Runs each job on its own thread.
struct ThreadExecutor;

impl Executor for ThreadExecutor {
    fn spawn(&self, mut task: Task) {
        std::thread::spawn(move || while task() {});
    }
}

#[derive(Clone)]
struct AppState {
    session: Arc<Session>,
    token: Arc<str>,
    hosts: Arc<[String]>,
    origins: Arc<[String]>,
}

pub struct Server {
    listener: TcpListener,
    addr: SocketAddr,
    state: AppState,
}

impl Server {
    pub async fn bind(opts: ServeOptions) -> anyhow::Result<Self> {
        let provider = FsProvider::new(&opts.roots, &opts.files)?;
        let session = Arc::new(Session::new("server", provider, ThreadExecutor));
        for f in &opts.files {
            let r = session.dispatch(Request::Open {
                source: OpenSource::Path {
                    path: f.to_string_lossy().into_owned(),
                },
                password: None,
                recover: false,
            });
            if let Response::Err(e) = r.response {
                eprintln!("pdfi: cannot open {}: {}", f.display(), e.message);
            }
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, opts.port)).await?;
        let addr = listener.local_addr()?;
        let port = addr.port();
        let mut token = [0u8; 32];
        getrandom::fill(&mut token).map_err(|e| anyhow::anyhow!("random token: {e}"))?;
        let token: String = token.iter().map(|b| format!("{b:02x}")).collect();
        let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
        let origins: Vec<String> = hosts
            .iter()
            .map(|h| format!("http://{h}"))
            .chain(opts.dev_origins)
            .collect();
        Ok(Self {
            listener,
            addr,
            state: AppState {
                session,
                token: token.into(),
                hosts: hosts.into(),
                origins: origins.into(),
            },
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn token(&self) -> &str {
        &self.state.token
    }

    /// URL to open in the browser (token in the fragment, never sent to the
    /// server in HTTP requests).
    pub fn url(&self) -> String {
        format!("http://{}/#token={}", self.addr, self.state.token)
    }

    pub async fn run(
        self,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .fallback(get(static_handler))
            .layer(middleware::from_fn_with_state(
                self.state.clone(),
                check_host,
            ))
            .with_state(self.state);
        axum::serve(self.listener, app)
            .with_graceful_shutdown(shutdown)
            .await?;
        Ok(())
    }
}

fn reject(status: StatusCode, msg: &'static str) -> HttpResponse {
    (status, msg).into_response()
}

async fn check_host(State(st): State<AppState>, req: HttpRequest, next: Next) -> HttpResponse {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    match host {
        Some(h) if st.hosts.iter().any(|a| a == h) => next.run(req).await,
        _ => reject(StatusCode::FORBIDDEN, "bad Host"),
    }
}

/// Constant-time comparison (the token is secret).
fn token_eq(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

async fn ws_handler(
    State(st): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> HttpResponse {
    let origin = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok());
    if !origin.is_some_and(|o| st.origins.iter().any(|a| a == o)) {
        return reject(StatusCode::FORBIDDEN, "bad Origin");
    }
    let protocols = headers
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim);
    let mut speaks = false;
    let mut authed = false;
    for p in protocols {
        if p == SUBPROTOCOL {
            speaks = true;
        } else if let Some(t) = p.strip_prefix(TOKEN_PREFIX) {
            authed |= token_eq(t, &st.token);
        }
    }
    if !authed {
        return reject(StatusCode::UNAUTHORIZED, "missing or wrong token");
    }
    if !speaks {
        return reject(StatusCode::BAD_REQUEST, "unsupported subprotocol");
    }
    let session = st.session.clone();
    ws.protocols([SUBPROTOCOL])
        .max_message_size(16 << 20)
        .on_upgrade(move |socket| connection(socket, session))
}

fn text(msg: &ServerMessage) -> Message {
    Message::Text(
        serde_json::to_string(msg)
            .expect("protocol messages serialize")
            .into(),
    )
}

fn error_message(id: u64, code: ErrorCode, message: impl Into<String>) -> Message {
    text(&ServerMessage::Response {
        id,
        response: Response::Err(ApiError::new(code, message)),
    })
}

/// Requests of one connection run in order on a worker thread; events and
/// replies are funneled through `out` to the socket.
async fn connection(mut socket: WebSocket, session: Arc<Session>) {
    let (out, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let sub = {
        let out = out.clone();
        session.subscribe(move |ev| {
            let _ = out.send(text(&ServerMessage::Event { event: ev.clone() }));
        })
    };
    let cancelled: Arc<Mutex<HashSet<u64>>> = Arc::default();
    let (req_tx, req_rx) = std::sync::mpsc::channel::<ClientMessage>();
    let worker = {
        let (session, out, cancelled) = (session.clone(), out.clone(), cancelled.clone());
        std::thread::spawn(move || {
            for m in req_rx {
                let skip = cancelled
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&m.id);
                if skip {
                    let _ = out.send(error_message(m.id, ErrorCode::Cancelled, "cancelled"));
                    continue;
                }
                let (frames, msg) = session.dispatch(m.request).into_frames(m.id);
                for f in frames {
                    let _ = out.send(Message::Binary(f.encode().into()));
                }
                if out.send(text(&msg)).is_err() {
                    break;
                }
            }
        })
    };
    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<ClientMessage>(&t) {
                    Ok(ClientMessage { id, request: Request::Cancel { request } }) => {
                        let mut c = cancelled.lock().unwrap_or_else(|e| e.into_inner());
                        if c.len() > 10_000 {
                            c.clear();
                        }
                        c.insert(request);
                        drop(c);
                        let _ = out.send(text(&ServerMessage::Response {
                            id,
                            response: Response::Ok(pdfi_api::ResponseBody::Empty),
                        }));
                    }
                    Ok(m) => {
                        if req_tx.send(m).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = out.send(error_message(0, ErrorCode::BadRequest, format!("malformed message: {e}")));
                    }
                },
                Some(Ok(Message::Binary(_))) => {
                    let _ = out.send(error_message(0, ErrorCode::BadRequest, "unexpected binary message"));
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
            Some(m) = out_rx.recv() => {
                if socket.send(m).await.is_err() {
                    break;
                }
            }
        }
    }
    session.unsubscribe(sub);
    drop(req_tx);
    // The worker exits once the queue is drained; do not block the runtime.
    tokio::task::spawn_blocking(move || {
        let _ = worker.join();
    });
}

// `wasm-unsafe-eval`: the same UI also runs the wasm backend when opened
// without a token.
const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; \
                   img-src 'self' data: blob:; \
                   style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; \
                   frame-ancestors 'none'";

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><title>pdfi</title>\
<p>The web UI is not built. Run <code>npm ci &amp;&amp; npm run build</code> in \
<code>apps/web</code>, then rebuild <code>pdfi</code>.</p>";

async fn static_handler(uri: Uri) -> HttpResponse {
    let path = uri.path().trim_start_matches('/');
    let (file, path) = match Assets::get(path) {
        Some(f) if !path.is_empty() => (Some(f), path),
        // Client-side routes fall back to the app shell.
        _ if path.is_empty() || !path.contains('.') => (Assets::get("index.html"), "index.html"),
        _ => return reject(StatusCode::NOT_FOUND, "not found"),
    };
    let (body, mime) = match file {
        Some(f) => (
            Body::from(f.data.into_owned()),
            f.metadata.mimetype().to_owned(),
        ),
        None if path == "index.html" => (Body::from(NOT_BUILT), "text/html".to_owned()),
        None => return reject(StatusCode::NOT_FOUND, "not found"),
    };
    let mut res = HttpResponse::new(body);
    let h = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&mime) {
        h.insert(header::CONTENT_TYPE, v);
    }
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let cache = if path == "index.html" {
        "no-store"
    } else {
        "public, max-age=31536000, immutable"
    };
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}
