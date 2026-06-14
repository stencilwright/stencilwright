//! Loopback RPC tests: spawn a fake daemon UnixListener that records
//! incoming requests and replies with canned values, then drive the
//! `Session`/`Page` client against it. Verifies the wire protocol +
//! the client's parse/round-trip behavior end-to-end without needing
//! Chrome or playwright.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use stencil_browser::Session;
use stencil_browser::rpc::{Request, Response};
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

/// A fake daemon: accepts connections, records each request, replies
/// with whatever `responder` returns. The recorded requests are
/// available via [`FakeDaemon::requests`].
struct FakeDaemon {
    requests: Arc<Mutex<Vec<Request>>>,
    _task: tokio::task::JoinHandle<()>,
}

impl FakeDaemon {
    async fn start(
        sock: &std::path::Path,
        responder: impl Fn(&Request) -> Value + Send + Sync + 'static,
    ) -> Self {
        let listener = UnixListener::bind(sock).expect("bind fake daemon socket");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_for_task = requests.clone();
        let responder = Arc::new(responder);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let recs = requests_for_task.clone();
                let resp = responder.clone();
                tokio::spawn(handle_client(stream, recs, resp));
            }
        });
        Self {
            requests,
            _task: task,
        }
    }

    async fn requests(&self) -> Vec<Request> {
        self.requests.lock().await.clone()
    }
}

async fn handle_client(
    stream: UnixStream,
    requests: Arc<Mutex<Vec<Request>>>,
    responder: Arc<dyn Fn(&Request) -> Value + Send + Sync>,
) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                let req: Request = match serde_json::from_str(&line) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                requests.lock().await.push(req.clone());
                let result = responder(&req);
                let resp = Response::ok(req.id, result);
                let mut out = serde_json::to_string(&resp).unwrap();
                out.push('\n');
                if write_half.write_all(out.as_bytes()).await.is_err() {
                    return;
                }
            }
        }
    }
}

#[tokio::test]
async fn page_methods_send_expected_requests() {
    use std::collections::BTreeMap;

    use stencil_core::{MaskConfig, Place, Signature, ValuesConfig};

    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let server = FakeDaemon::start(&sock, |req| match req.op.as_str() {
        "url" => json!("https://example.com/"),
        "url_template" => json!("https://example.com/u/{example_username}/"),
        "locator_count" => json!(7),
        "locator_visible_count" => json!(3),
        "content" => json!("<html><body><h1>Example Domain</h1></body></html>"),
        "dump_masked" => json!("<html>masked</html>"),
        _ => Value::Null,
    })
    .await;

    let session = Session::connect(&sock).await.unwrap();
    let page = session.page();
    let values = ValuesConfig {
        entries: BTreeMap::from([
            (
                "example_username".to_string(),
                "secret://1password/vault-id/example-item/username".to_string(),
            ),
            (
                "example_password".to_string(),
                "secret://1password/vault-id/example-item/password".to_string(),
            ),
        ]),
    };
    let place = Place {
        name: "login".into(),
        url: None,
        from: None,
        via: None,
        interactive: false,
        submit: None,
        signature: Signature::default(),
        completion: None,
        redirect: None,
        elements: vec![],
    };

    page.goto("https://example.com/").await.unwrap();
    page.goto_template("https://example.com/u/{example_username}/", &values)
        .await
        .unwrap();
    page.click("button#submit").await.unwrap();
    page.fill("input[name='username']", "alice").await.unwrap();
    page.fill_ref("input[name='password']", "{example_password}", &values)
        .await
        .unwrap();
    page.select_option("select#country", "us").await.unwrap();
    page.wait_for("h1", Duration::from_millis(5_000))
        .await
        .unwrap();

    assert_eq!(page.url().await.unwrap(), "https://example.com/");
    assert_eq!(
        page.url_template(&values).await.unwrap(),
        "https://example.com/u/{example_username}/"
    );
    assert_eq!(page.locator_count("li").await.unwrap(), 7);
    assert_eq!(page.locator_visible_count("li").await.unwrap(), 3);
    let masked = page
        .dump_masked(&MaskConfig::default(), &[], Some(&place), &values)
        .await
        .unwrap();
    assert_eq!(masked.0, "<html>masked</html>");

    let reqs = server.requests().await;
    assert_eq!(reqs.len(), 12);

    // Argument shape per op.
    assert_eq!(reqs[0].op, "goto");
    assert_eq!(reqs[0].args["url"], "https://example.com/");

    assert_eq!(reqs[1].op, "goto_template");
    assert_eq!(
        reqs[1].args["url"],
        "https://example.com/u/{example_username}/"
    );
    assert_eq!(
        reqs[1].args["values"]["example_username"],
        "secret://1password/vault-id/example-item/username"
    );

    assert_eq!(reqs[2].op, "click");
    assert_eq!(reqs[2].args["selector"], "button#submit");

    assert_eq!(reqs[3].op, "fill");
    assert_eq!(reqs[3].args["selector"], "input[name='username']");
    assert_eq!(reqs[3].args["value"], "alice");

    assert_eq!(reqs[4].op, "fill_ref");
    assert_eq!(reqs[4].args["selector"], "input[name='password']");
    assert_eq!(reqs[4].args["value_ref"], "{example_password}");
    assert_eq!(
        reqs[4].args["values"]["example_password"],
        "secret://1password/vault-id/example-item/password"
    );

    assert_eq!(reqs[5].op, "select_option");
    assert_eq!(reqs[5].args["selector"], "select#country");
    assert_eq!(reqs[5].args["value"], "us");

    assert_eq!(reqs[6].op, "wait_for");
    assert_eq!(reqs[6].args["selector"], "h1");
    assert_eq!(reqs[6].args["timeout_ms"], 5_000);

    assert_eq!(reqs[7].op, "url");
    assert_eq!(reqs[8].op, "url_template");
    assert_eq!(reqs[9].op, "locator_count");
    assert_eq!(reqs[10].op, "locator_visible_count");
    assert_eq!(reqs[10].args["selector"], "li");
    assert_eq!(reqs[11].op, "dump_masked");
    assert_eq!(reqs[11].args["place"]["name"], "login");
    assert_eq!(
        reqs[11].args["values"]["example_username"],
        "secret://1password/vault-id/example-item/username"
    );

    // IDs are monotonic per session.
    let ids: Vec<u64> = reqs.iter().map(|r| r.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(ids, sorted, "ids should be unique and monotonic");
}

#[tokio::test]
async fn capture_runs_masking_pipeline() {
    use stencil_core::{MaskConfig, MaskInner, PatternRule};
    use stencil_mask::{MaskPolicy, ValueNameMap};

    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");
    let _server = FakeDaemon::start(&sock, |req| match req.op.as_str() {
        "content" => {
            json!("<html><body><h1>Example Domain</h1><p>Account 12345678 here.</p></body></html>")
        }
        _ => Value::Null,
    })
    .await;

    let session = Session::connect(&sock).await.unwrap();
    let page = session.page();

    let cfg = MaskConfig {
        mask: MaskInner {
            patterns: vec![PatternRule {
                name: "long_digits".into(),
                regex: r"[0-9]{8,}".into(),
            }],
            redact_selectors: vec![],
        },
        max_unmasked_chars: 200,
    };
    let policy = MaskPolicy::build(&cfg, &[]).unwrap();
    let place = stencil_core::Place {
        name: "x".into(),
        url: None,
        from: None,
        via: None,
        interactive: false,
        submit: None,
        signature: stencil_core::Signature::default(),
        completion: None,
        redirect: None,
        elements: vec![],
    };
    let effective = policy.for_place(&place);
    let masked = page.dump(&effective, &ValueNameMap::new()).await.unwrap();

    // Default-deny path: text content is replaced with [TEXT:N] and
    // the sensitive `12345678` does NOT pass through.
    assert!(
        masked.0.contains("[TEXT:"),
        "expected [TEXT:N], got {}",
        masked.0
    );
    assert!(
        !masked.0.contains("12345678"),
        "raw value leaked through capture: {}",
        masked.0,
    );
    // Structure preserved.
    assert!(masked.0.contains("<h1>"));
}

#[tokio::test]
async fn rpc_error_propagates_to_client() {
    let tmp = tempdir().unwrap();
    let sock = tmp.path().join("daemon.sock");

    // Server replies with an explicit error.
    let listener = UnixListener::bind(&sock).unwrap();
    let _server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    return;
                }
                let req: Request = serde_json::from_str(&line).unwrap();
                let resp = Response::err(req.id, "synthetic failure");
                let mut out = serde_json::to_string(&resp).unwrap();
                out.push('\n');
                let _ = write_half.write_all(out.as_bytes()).await;
            });
        }
    });

    let session = Session::connect(&sock).await.unwrap();
    let page = session.page();
    let err = page.goto("https://example.com/").await.unwrap_err();
    assert!(
        err.to_string().contains("synthetic failure"),
        "expected daemon error in client message, got: {err}",
    );
}
