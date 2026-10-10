//! Functions end to end, with a real Bun: set `PORCHLY_TEST_BUN` to a `bun` program to run
//! these (they're skipped otherwise, so the suite never downloads anything).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{Read, Write};

use porchly_engine::Engine;
use serde_json::{Value, json};

fn bun() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os("PORCHLY_TEST_BUN")?);
    path.is_file().then_some(path)
}

/// Sends one HTTP request; returns the status and the body.
fn http(port: u16, method: &str, path: &str, key: Option<&str>, body: &str) -> (u16, String) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let auth = key.map(|k| format!("Authorization: Bearer {k}\r\n")).unwrap_or_default();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status = out.split(' ').nth(1).unwrap().parse().unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    // Small bodies may come chunked; keep it simple for JSON answers.
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        let mut text = String::new();
        let mut rest = body;
        while let Some((size, after)) = rest.split_once("\r\n") {
            let n = usize::from_str_radix(size.trim(), 16).unwrap();
            if n == 0 {
                break;
            }
            text.push_str(&after[..n]);
            rest = &after[n + 2..];
        }
        text
    } else {
        body.to_owned()
    };
    (status, body)
}

#[test]
fn functions_end_to_end() {
    let Some(bun) = bun() else {
        eprintln!("skipped: set PORCHLY_TEST_BUN to a bun program to run the functions test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().join("p.db")).unwrap();
    e.use_bun(Some(bun));

    // Off until turned on.
    assert_eq!(e.execute("functions.status", &json!({})).unwrap()["enabled"], false);
    assert!(e.execute("function.create", &json!({"name": "hello"})).is_err(), "not available while off");
    let on = e.execute("functions.enable", &json!({})).unwrap();
    assert_eq!((on["enabled"].as_bool(), on["runtime"]["ready"].as_bool()), (Some(true), Some(true)));

    e.execute("sample.load", &json!({})).unwrap();
    let key = e.execute("api.key.create", &json!({"name": "site"})).unwrap()["secret"].as_str().unwrap().to_owned();
    let port = e.execute("api.start", &json!({})).unwrap()["port"].as_u64().unwrap() as u16;

    // The template answers, with sub-paths and the query string passed through.
    e.execute("function.create", &json!({"name": "hello"})).unwrap();
    let (status, body) = http(port, "GET", "/api/fn/hello/a/b?name=Ann", Some(&key), "");
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!((v["hello"].as_str(), v["path"].as_str()), (Some("Ann"), Some("/a/b")));

    // Needs a key unless set to "anyone".
    assert_eq!(http(port, "GET", "/api/fn/hello", None, "").0, 401);
    e.execute("function.settings", &json!({"name": "hello", "access": "public"})).unwrap();
    assert_eq!(http(port, "GET", "/api/fn/hello", None, "").0, 200);
    assert_eq!(http(port, "GET", "/api/fn/nope", Some(&key), "").0, 404);

    // The caller's key never reaches the function; who called does. Variables arrive.
    e.execute(
        "function.save",
        &json!({"name": "hello", "code": r#"
export default {
  async fetch(request) {
    console.log("called with", request.method);
    return Response.json({
      auth: request.headers.get("authorization"),
      key: request.headers.get("x-porchly-key-name"),
      write: request.headers.get("x-porchly-key-write"),
      body: await request.text(),
      mail: process.env.MAIL_KEY ?? null,
      home: process.env.SECRET_FROM_PORCHLY ?? null,
    }, { headers: { "cache-control": "max-age=60" } });
  },
};"#}),
    )
    .unwrap();
    e.execute("function.settings", &json!({"name": "hello", "access": "key", "env": {"MAIL_KEY": "m1"}})).unwrap();
    let (status, body) = http(port, "POST", "/api/fn/hello", Some(&key), "{\"x\":1}");
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["auth"], Value::Null);
    assert_eq!((v["key"].as_str(), v["write"].as_str()), (Some("site"), Some("false")));
    assert_eq!((v["body"].as_str(), v["mail"].as_str()), (Some("{\"x\":1}"), Some("m1")));
    let log = e.execute("function.logs", &json!({"name": "hello"})).unwrap();
    assert!(
        log["lines"].as_array().unwrap().iter().any(|l| l.as_str().unwrap().ends_with("out called with POST")),
        "{log}"
    );

    // Functions reach the tables with Porchly's internal key, including changes.
    e.execute("function.create", &json!({"name": "tables", "template": "tables"})).unwrap();
    let (status, body) = http(port, "GET", "/api/fn/tables", Some(&key), "");
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["tables"], json!(["recipes"]));
    assert!(!v["first_rows"].as_array().unwrap().is_empty());
    e.execute(
        "function.save",
        &json!({"name": "tables", "code": r#"
const h = { Authorization: `Bearer ${process.env.PORCHLY_TOKEN}`, "Content-Type": "application/json" };
export default {
  async fetch() {
    const r = await fetch(`${process.env.PORCHLY_URL}/api/recipes`, { method: "POST", headers: h, body: JSON.stringify({ name: "From a function" }) });
    return new Response(await r.text(), { status: r.status });
  },
};"#}),
    )
    .unwrap();
    let (status, body) = http(port, "POST", "/api/fn/tables", Some(&key), "");
    assert_eq!(status, 201, "{body}");
    let page = e.execute("row.list", &json!({"table": "recipes", "where": {"name": "From a function"}})).unwrap();
    assert_eq!(page["total"], 1);

    // Broken code: a clear error to the caller, Porchly carries on.
    e.execute("function.save", &json!({"name": "tables", "code": "export default { fetch( {"})).unwrap();
    let (status, body) = http(port, "GET", "/api/fn/tables", Some(&key), "");
    assert_eq!(status, 502, "{body}");
    assert!(body.contains("function_failed") && body.contains("couldn't start"), "{body}");
    e.execute(
        "function.save",
        &json!({"name": "tables", "code": "export default { fetch() { throw new Error('boom') } };"}),
    )
    .unwrap();
    let (status, body) = http(port, "GET", "/api/fn/tables", Some(&key), "");
    assert_eq!(status, 500, "{body}");
    assert!(body.contains("function_failed"), "{body}");
    e.execute("function.save", &json!({"name": "tables", "code": "export const nothing = 1;"})).unwrap();
    assert_eq!(http(port, "GET", "/api/fn/tables", Some(&key), "").0, 502);

    // Several calls at once each get their own Bun.
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let key = key.clone();
            std::thread::spawn(move || http(port, "GET", "/api/fn/hello", Some(&key), "").0)
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), 200);
    }

    // A time limit (Settings) stops a function that doesn't answer.
    e.execute("function.create", &json!({"name": "slow"})).unwrap();
    e.execute("function.save", &json!({"name": "slow", "code": "export default { async fetch() { await Bun.sleep(60000); return new Response('late'); } };"}))
        .unwrap();
    e.execute("settings.set", &json!({"function_timeout": 10})).unwrap();
    let started = std::time::Instant::now();
    let (status, body) = http(port, "GET", "/api/fn/slow", Some(&key), "");
    assert_eq!(status, 504, "{body}");
    assert!(body.contains("function_timeout") && body.contains("10 seconds"), "{body}");
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    e.execute("settings.set", &json!({"function_timeout": 0})).unwrap();

    // The app's test panel.
    let tried = e.execute("function.call", &json!({"name": "hello", "method": "post", "body": {"a": 1}})).unwrap();
    assert_eq!(tried["status"], 200);
    assert!(tried["body"].as_str().unwrap().contains("Porchly functions"), "{tried}");

    // Listed in the docs; gone once turned off.
    let docs = e.execute("api.docs", &json!({})).unwrap()["html"].as_str().unwrap().to_owned();
    assert!(docs.contains("/api/fn/hello"));
    e.execute("functions.disable", &json!({})).unwrap();
    assert_eq!(http(port, "GET", "/api/fn/hello", Some(&key), "").0, 404);
    assert!(dir.path().join("functions/hello/main.ts").is_file(), "turning off keeps the code");
}
