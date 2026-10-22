use std::path::PathBuf;

use futures_util::{SinkExt, StreamExt};
use pdfi_api::{BinaryFrame, ErrorCode, PROTOCOL_VERSION, Response, ResponseBody, ServerMessage};
use pdfi_server::{ServeOptions, Server};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .canonicalize()
        .unwrap()
}

struct Running {
    port: u16,
    token: String,
    _stop: tokio::sync::oneshot::Sender<()>,
}

async fn start(opts: ServeOptions) -> Running {
    let server = Server::bind(opts).await.unwrap();
    let port = server.addr().port();
    let token = server.token().to_owned();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    Running {
        port,
        token,
        _stop: tx,
    }
}

async fn raw_http(port: u16, req: String) -> String {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    String::from_utf8_lossy(&out).into_owned()
}

fn get(host: &str) -> String {
    format!("GET / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n")
}

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(
    port: u16,
    origin: Option<&str>,
    protocols: &str,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://127.0.0.1:{port}/ws")
        .into_client_request()
        .unwrap();
    let h = req.headers_mut();
    if let Some(o) = origin {
        h.insert("Origin", o.parse().unwrap());
    }
    h.insert("Sec-WebSocket-Protocol", protocols.parse().unwrap());
    tokio_tungstenite::connect_async(req)
        .await
        .map(|(ws, _)| ws)
}

async fn good(r: &Running) -> Ws {
    connect(
        r.port,
        Some(&format!("http://127.0.0.1:{}", r.port)),
        &format!("pdfi.v1, pdfi.token.{}", r.token),
    )
    .await
    .unwrap()
}

/// Send a request and wait for its response, collecting binary parts.
async fn call(ws: &mut Ws, id: u64, request: serde_json::Value) -> (Response, Vec<BinaryFrame>) {
    let msg = serde_json::json!({ "id": id, "request": request });
    ws.send(Message::text(msg.to_string())).await.unwrap();
    let mut frames = Vec::new();
    while let Some(m) = ws.next().await {
        match m.unwrap() {
            Message::Binary(b) => {
                let f = BinaryFrame::decode(&b).unwrap();
                if f.id == id {
                    frames.push(f);
                }
            }
            Message::Text(t) => {
                if let ServerMessage::Response { id: rid, response } =
                    serde_json::from_str(&t).unwrap()
                {
                    if rid == id {
                        return (response, frames);
                    }
                }
            }
            _ => {}
        }
    }
    panic!("connection closed");
}

#[tokio::test]
async fn host_header_is_checked() {
    let r = start(ServeOptions::default()).await;
    let ok = raw_http(r.port, get(&format!("127.0.0.1:{}", r.port))).await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    assert!(ok.contains("content-security-policy"));
    let ok = raw_http(r.port, get(&format!("localhost:{}", r.port))).await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
    let bad = raw_http(r.port, get(&format!("evil.example:{}", r.port))).await;
    assert!(bad.starts_with("HTTP/1.1 403"), "{bad}");
    let bad = raw_http(r.port, "GET / HTTP/1.1\r\nConnection: close\r\n\r\n".into()).await;
    assert!(bad.starts_with("HTTP/1.1 4"), "{bad}");
}

#[tokio::test]
async fn websocket_requires_origin_and_token() {
    let r = start(ServeOptions::default()).await;
    let origin = format!("http://127.0.0.1:{}", r.port);
    let with_token = format!("pdfi.v1, pdfi.token.{}", r.token);
    assert!(connect(r.port, None, &with_token).await.is_err());
    assert!(
        connect(r.port, Some("http://evil.example"), &with_token)
            .await
            .is_err()
    );
    assert!(connect(r.port, Some(&origin), "pdfi.v1").await.is_err());
    let wrong = format!("pdfi.v1, pdfi.token.{}", "0".repeat(r.token.len()));
    assert!(connect(r.port, Some(&origin), &wrong).await.is_err());

    let mut ws = good(&r).await;
    let (res, _) = call(
        &mut ws,
        1,
        serde_json::json!({ "op": "hello", "protocol": PROTOCOL_VERSION }),
    )
    .await;
    assert!(matches!(res, Response::Ok(ResponseBody::Hello(h)) if h.filesystem));
}

#[tokio::test]
async fn files_outside_roots_are_forbidden() {
    let fx = fixtures();
    let r = start(ServeOptions {
        files: vec![fx.join("base.pdf")],
        ..Default::default()
    })
    .await;
    let mut ws = good(&r).await;

    // Opened at startup.
    let (res, _) = call(&mut ws, 1, serde_json::json!({ "op": "docs" })).await;
    let Response::Ok(ResponseBody::Docs { docs }) = res else {
        panic!("{res:?}")
    };
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].name, "base.pdf");

    // Re-opening the same path returns the same document.
    let open = |p: PathBuf| {
        serde_json::json!({
            "op": "open",
            "source": { "kind": "path", "path": p.to_string_lossy() },
            "password": null,
            "recover": false,
        })
    };
    let (res, _) = call(&mut ws, 2, open(fx.join("base.pdf"))).await;
    assert!(matches!(res, Response::Ok(ResponseBody::Doc(s)) if s.info.doc == docs[0].doc));
    // A sibling file is not allowed, neither via `..` tricks.
    let (res, _) = call(&mut ws, 3, open(fx.join("objstm.pdf"))).await;
    assert!(matches!(res, Response::Err(e) if e.code == ErrorCode::Forbidden));
    let (res, _) = call(&mut ws, 4, open(fx.join("../fixtures/objstm.pdf"))).await;
    assert!(matches!(res, Response::Err(e) if e.code == ErrorCode::Forbidden));
    let (res, _) = call(
        &mut ws,
        5,
        serde_json::json!({ "op": "listDir", "path": fx.to_string_lossy() }),
    )
    .await;
    assert!(matches!(res, Response::Err(e) if e.code == ErrorCode::Forbidden));
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_cannot_escape_roots() {
    let dir = std::env::temp_dir().join(format!("pdfi-server-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("root")).unwrap();
    std::os::unix::fs::symlink(fixtures().join("base.pdf"), dir.join("root/link.pdf")).unwrap();
    std::fs::copy(fixtures().join("base.pdf"), dir.join("root/inside.pdf")).unwrap();

    let r = start(ServeOptions {
        roots: vec![dir.join("root")],
        ..Default::default()
    })
    .await;
    let mut ws = good(&r).await;
    let open = |p: PathBuf| {
        serde_json::json!({
            "op": "open",
            "source": { "kind": "path", "path": p.to_string_lossy() },
            "password": null,
            "recover": false,
        })
    };
    let (res, _) = call(&mut ws, 1, open(dir.join("root/link.pdf"))).await;
    assert!(matches!(res, Response::Err(e) if e.code == ErrorCode::Forbidden));
    let (res, _) = call(&mut ws, 2, open(dir.join("root/inside.pdf"))).await;
    let Response::Ok(ResponseBody::Doc(s)) = res else {
        panic!("{res:?}")
    };

    let (res, _) = call(
        &mut ws,
        3,
        serde_json::json!({ "op": "listDir", "path": dir.join("root").to_string_lossy() }),
    )
    .await;
    let Response::Ok(ResponseBody::Dir(d)) = res else {
        panic!("{res:?}")
    };
    assert_eq!(d.entries.len(), 2);
    assert!(d.parent.is_none());

    // Binary parts arrive before the response that refers to them.
    let (res, frames) = call(
        &mut ws,
        4,
        serde_json::json!({ "op": "rawBytes", "doc": s.info.doc, "offset": 0, "len": 8 }),
    )
    .await;
    assert!(matches!(res, Response::Ok(ResponseBody::Bytes(b)) if b.part == 0 && b.len == 8));
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data, b"%PDF-1.7");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn malformed_messages_get_errors() {
    let r = start(ServeOptions::default()).await;
    let mut ws = good(&r).await;
    ws.send(Message::text("{not json")).await.unwrap();
    let Some(Ok(Message::Text(t))) = ws.next().await else {
        panic!()
    };
    let m: ServerMessage = serde_json::from_str(&t).unwrap();
    assert!(matches!(
        m,
        ServerMessage::Response { id: 0, response: Response::Err(e) } if e.code == ErrorCode::BadRequest
    ));
}

/// The conformance scenario over the WebSocket must match the Rust API
/// (`tests/conformance/expected.json`).
#[tokio::test]
async fn conformance() {
    use pdfi_api::Event;
    use pdfi_api::conformance::{self, Step};

    let r = start(ServeOptions {
        roots: vec![fixtures()],
        ..Default::default()
    })
    .await;
    let mut ws = good(&r).await;
    let mut events: Vec<Event> = Vec::new();
    let mut next_id = 0u64;
    let mut out = Vec::new();

    // Like `call`, but keeps the events that arrive meanwhile.
    async fn send(
        ws: &mut Ws,
        id: u64,
        request: &pdfi_api::Request,
        events: &mut Vec<Event>,
    ) -> (Response, Vec<Vec<u8>>) {
        let msg = pdfi_api::ClientMessage {
            id,
            request: request.clone(),
        };
        ws.send(Message::text(serde_json::to_string(&msg).unwrap()))
            .await
            .unwrap();
        let mut blobs = Vec::new();
        while let Some(m) = ws.next().await {
            match m.unwrap() {
                Message::Binary(b) => {
                    let f = BinaryFrame::decode(&b).unwrap();
                    assert_eq!(f.id, id);
                    assert_eq!(f.part as usize, blobs.len());
                    blobs.push(f.data);
                }
                Message::Text(t) => match serde_json::from_str(&t).unwrap() {
                    ServerMessage::Response { id: rid, response } => {
                        assert_eq!(rid, id);
                        return (response, blobs);
                    }
                    ServerMessage::Event { event } => events.push(event),
                },
                _ => {}
            }
        }
        panic!("connection closed");
    }

    for step in conformance::scenario() {
        next_id += 1;
        let entry = match step {
            Step::Request(req) => {
                let (res, blobs) = send(&mut ws, next_id, &req, &mut events).await;
                conformance::entry(&res, &blobs, &[])
            }
            Step::Open {
                file,
                password,
                recover,
            } => {
                let req = pdfi_api::Request::Open {
                    source: pdfi_api::OpenSource::Path {
                        path: fixtures().join(file).to_string_lossy().into_owned(),
                    },
                    password,
                    recover,
                };
                let (res, blobs) = send(&mut ws, next_id, &req, &mut events).await;
                conformance::entry(&res, &blobs, &[])
            }
            Step::Job(req) => {
                events.clear();
                let (res, blobs) = send(&mut ws, next_id, &req, &mut events).await;
                let job = conformance::started_job(&res).expect("job started");
                // Jobs run on their own thread; wait for the last event.
                while !events
                    .iter()
                    .any(|e| conformance::job_event(e, job) == Some(true))
                {
                    match ws.next().await.unwrap().unwrap() {
                        Message::Text(t) => {
                            if let ServerMessage::Event { event } =
                                serde_json::from_str(&t).unwrap()
                            {
                                events.push(event);
                            }
                        }
                        m => panic!("unexpected {m:?}"),
                    }
                }
                let evs: Vec<Event> = events
                    .iter()
                    .filter(|e| conformance::job_event(e, job).is_some())
                    .cloned()
                    .collect();
                conformance::entry(&res, &blobs, &evs)
            }
        };
        out.push(entry);
    }
    conformance::check("server", out, false);
}

/// A one-page PDF whose content is `content`.
fn one_page(content: &[u8]) -> Vec<u8> {
    let objs: [Vec<u8>; 4] = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 600 800] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>".to_vec(),
        [
            format!("<< /Length {} >>\nstream\n", content.len()).as_bytes(),
            content,
            b"\nendstream",
        ]
        .concat(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(o);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(b"xref\n0 5\n0000000000 65535 f\r\n");
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// Read messages until every id in `ids` has its response.
async fn responses(ws: &mut Ws, ids: &[u64]) -> Vec<(u64, Response, Vec<BinaryFrame>)> {
    let mut frames: Vec<BinaryFrame> = Vec::new();
    let mut out = Vec::new();
    while out.len() < ids.len() {
        match ws.next().await.expect("connection closed").unwrap() {
            Message::Binary(b) => frames.push(BinaryFrame::decode(&b).unwrap()),
            Message::Text(t) => {
                if let ServerMessage::Response { id, response } = serde_json::from_str(&t).unwrap()
                {
                    if ids.contains(&id) {
                        let mine = frames.iter().filter(|f| f.id == id).cloned().collect();
                        out.push((id, response, mine));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[tokio::test]
async fn renders_run_in_parallel_and_can_be_cancelled() {
    let dir = std::env::temp_dir().join(format!("pdfi-render-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let heavy = dir.join("heavy.pdf");
    std::fs::write(&heavy, one_page(&b"0 0 600 800 re f\n".repeat(400_000))).unwrap();
    std::fs::copy(fixtures().join("render.pdf"), dir.join("render.pdf")).unwrap();
    let r = start(ServeOptions {
        files: vec![dir.join("render.pdf"), heavy.clone()],
        ..Default::default()
    })
    .await;
    let mut ws = good(&r).await;
    let (res, _) = call(&mut ws, 1, serde_json::json!({ "op": "docs" })).await;
    let Response::Ok(ResponseBody::Docs { docs }) = res else {
        panic!("{res:?}")
    };
    let doc = |name: &str| docs.iter().find(|d| d.name == name).unwrap().doc;

    // Four tiles at once: each comes back with its bitmap.
    let ids = [10, 11, 12, 13];
    for (k, id) in ids.iter().enumerate() {
        let msg = serde_json::json!({ "id": id, "request": {
            "op": "render", "doc": doc("render.pdf"), "content": { "kind": "page", "index": 1 },
            "scale": 1, "until": null, "layer": { "kind": "content" },
            "tile": { "x": (k % 2) * 200, "y": (k / 2) * 200, "width": 200, "height": 200 },
        }});
        ws.send(Message::text(msg.to_string())).await.unwrap();
    }
    for (id, res, frames) in responses(&mut ws, &ids).await {
        let Response::Ok(ResponseBody::Bitmap(b)) = res else {
            panic!("{id}: {res:?}")
        };
        assert_eq!((b.rect.width, b.rect.height), (200, 200));
        assert_eq!(frames[b.part as usize].data.len(), 200 * 200 * 4);
    }

    // A long render stops when cancelled.
    let msg = serde_json::json!({ "id": 20, "request": {
        "op": "render", "doc": doc("heavy.pdf"), "content": { "kind": "page", "index": 0 },
        "scale": 2, "tile": null, "until": null, "layer": { "kind": "content" },
    }});
    ws.send(Message::text(msg.to_string())).await.unwrap();
    let cancel = serde_json::json!({ "id": 21, "request": { "op": "cancel", "request": 20 } });
    ws.send(Message::text(cancel.to_string())).await.unwrap();
    let got = responses(&mut ws, &[20, 21]).await;
    let (_, res, _) = got.iter().find(|(id, ..)| *id == 20).unwrap();
    assert!(
        matches!(res, Response::Err(e) if e.code == ErrorCode::Cancelled),
        "{res:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
