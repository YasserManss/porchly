//! The porch light end to end, through a real Cloudflare quick tunnel. Needs the internet:
//! set `PORCHLY_TEST_CLOUDFLARED` to a `cloudflared` program to run it (skipped otherwise).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::time::{Duration, Instant};

use porchly_engine::Engine;
use serde_json::{Value, json};

/// One request through the public link; returns status and body.
fn call(method: &str, url: &str, key: &str, body: Option<&str>) -> (u16, String) {
    let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
    let req = ureq::http::Request::builder()
        .method(method)
        .uri(url)
        .header("authorization", format!("Bearer {key}"))
        .header("content-type", "application/json")
        .body(body.unwrap_or_default().to_owned())
        .unwrap();
    match agent.run(req) {
        Ok(res) => {
            let status = res.status().as_u16();
            (status, res.into_body().read_to_string().unwrap_or_default())
        }
        Err(e) => (0, e.to_string()),
    }
}

/// A new quick tunnel can take a few seconds before Cloudflare routes to it.
fn call_until_routed(method: &str, url: &str, key: &str, body: Option<&str>) -> (u16, String) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        let (status, text) = call(method, url, key, body);
        if (status != 0 && status != 530 && status != 502 && status != 404) || Instant::now() > deadline {
            return (status, text);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn the_porch_light_end_to_end() {
    let Some(program) = std::env::var_os("PORCHLY_TEST_CLOUDFLARED").map(std::path::PathBuf::from) else {
        eprintln!("skipped: set PORCHLY_TEST_CLOUDFLARED to a cloudflared program to run the porch light test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().join("p.db")).unwrap();
    e.use_memory_secrets();
    e.use_cloudflared(Some(program));
    e.execute("sample.load", &json!({})).unwrap();
    let writer = e.execute("api.key.create", &json!({"name": "w", "write": true})).unwrap()["secret"]
        .as_str()
        .unwrap()
        .to_owned();

    assert_eq!(e.execute("tunnel.status", &json!({})).unwrap()["state"], "off", "off at start");
    e.execute("tunnel.start", &json!({})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let url = loop {
        let s = e.execute("tunnel.status", &json!({})).unwrap();
        if s["state"] == "on" {
            break s["url"].as_str().unwrap().to_owned();
        }
        assert_ne!(s["state"], "failed", "{s}");
        assert!(Instant::now() < deadline, "no link after 60 s: {s}");
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(url.starts_with("https://") && url.ends_with(".trycloudflare.com"), "{url}");

    // Reading works from the internet; changes don't, until allowed.
    let (status, body) = call_until_routed("GET", &format!("{url}/api/recipes?limit=1"), &writer, None);
    assert_eq!(status, 200, "{body}");
    let (status, body) = call("POST", &format!("{url}/api/recipes"), &writer, Some(r#"{"name":"From the internet"}"#));
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("public link"), "{body}");
    e.execute("tunnel.allow_changes", &json!({"allow": true})).unwrap();
    let (status, body) = call("POST", &format!("{url}/api/recipes"), &writer, Some(r#"{"name":"From the internet"}"#));
    assert_eq!(status, 201, "{body}");
    let docs = call("GET", &format!("{url}/docs"), &writer, None).1;
    assert!(docs.contains(&format!("{url}/api/recipes")), "the guide uses the https link");

    assert_eq!(e.execute("api.status", &json!({})).unwrap()["running"], true, "the API came on with the light");

    // Off: the link stops working.
    e.execute("tunnel.stop", &json!({})).unwrap();
    assert_eq!(e.execute("tunnel.status", &json!({})).unwrap()["state"], "off");
    std::thread::sleep(Duration::from_secs(2));
    let (status, _) = call("GET", &format!("{url}/api/recipes"), &writer, None);
    assert_ne!(status, 200, "the link is gone");
    let v: Value = e.execute("tunnel.status", &json!({})).unwrap();
    assert_eq!(v["allow_changes"], true, "the choice is remembered");

    // Turning the API off turns the light off too.
    e.execute("tunnel.start", &json!({})).unwrap();
    e.execute("api.stop", &json!({})).unwrap();
    assert_eq!(e.execute("tunnel.status", &json!({})).unwrap()["state"], "off");
}

/// Your own stable address, with a stand-in for cloudflared (no internet, no Cloudflare
/// account): everything on Porchly's side of a tunnel run with a token.
#[cfg(unix)]
#[test]
fn your_own_address_with_a_token() {
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    // A token shaped like Cloudflare's: base64 of {"a","t","s"}. Made up.
    let token = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMTExMTExMTEtMjIyMi0zMzMzLTQ0NDQtNTU1NTU1NTU1NTU1IiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQzF6WldOeVpYUT0ifQ==";
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake-cloudflared");
    std::fs::write(
        &fake,
        r#"#!/bin/sh
[ -n "$TUNNEL_TOKEN" ] || { echo 'ERR Provided Tunnel token is not valid.' >&2; exit 1; }
echo 'INF Updated to new configuration config="{\"ingress\":[{\"hostname\":\"api.example.com\",\"service\":\"http://localhost:11920\"},{\"service\":\"http_status:404\"}]}" version=1' >&2
echo 'INF Registered tunnel connection connIndex=0' >&2
sleep 30
"#,
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

    let project = dir.path().join("p.db");
    let e = Engine::open(&project).unwrap();
    e.use_memory_secrets();
    e.use_cloudflared(Some(fake.clone()));
    e.execute("sample.load", &json!({})).unwrap();
    let writer = e.execute("api.key.create", &json!({"name": "w", "write": true})).unwrap()["secret"]
        .as_str()
        .unwrap()
        .to_owned();

    let status = e.execute("tunnel.status", &json!({})).unwrap();
    assert_eq!(
        (status["has_token"].as_bool(), status["service"].as_str()),
        (Some(false), Some("http://localhost:11920"))
    );
    assert!(e.execute("tunnel.start", &json!({"own": true})).is_err(), "no token yet");
    assert!(e.execute("tunnel.token.set", &json!({"token": "not a token"})).is_err());
    assert!(e.execute("tunnel.token.set", &json!({})).is_err());

    // Pasting the whole command Cloudflare shows works too.
    let saved =
        e.execute("tunnel.token.set", &json!({"token": format!("sudo cloudflared service install {token}")})).unwrap();
    assert_eq!((saved["has_token"].as_bool(), saved["token_lasting"].as_bool()), (Some(true), Some(false)));
    assert!(!format!("{saved}").contains(token), "the token is never given back");

    // With a token saved, the porch light uses your own address.
    e.execute("tunnel.start", &json!({})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        let s = e.execute("tunnel.status", &json!({})).unwrap();
        if s["state"] == "on" {
            break s;
        }
        assert!(s["state"] != "failed" && Instant::now() < deadline, "{s}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!((status["url"].as_str(), status["own"].as_bool()), (Some("https://api.example.com"), Some(true)));
    assert!(status["problem"].is_null());

    // Porchly's public door is on the fixed port the tunnel sends to, and it's read-only.
    let request = |method: &str, body: &str| {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", 11920)).unwrap();
        write!(
            s,
            "{method} /api/recipes HTTP/1.1\r\nHost: api.example.com\r\nAuthorization: Bearer {writer}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out.split(' ').nth(1).unwrap().to_owned()
    };
    assert_eq!(request("GET", ""), "200");
    assert_eq!(request("POST", r#"{"name":"From the internet"}"#), "403", "read-only until changes are allowed");

    // The token is in the keychain only: not in the project, its backups or the log.
    e.execute("backup.create", &json!({})).unwrap();
    fn holds(dir: &std::path::Path, needle: &[u8]) -> Option<std::path::PathBuf> {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = holds(&path, needle) {
                    return Some(found);
                }
            } else if std::fs::read(&path).unwrap_or_default().windows(needle.len()).any(|w| w == needle) {
                return Some(path);
            }
        }
        None
    }
    assert_eq!(holds(dir.path(), token.as_bytes()), None, "the token was written to a file");

    // Forgetting the token turns the light off.
    let cleared = e.execute("tunnel.token.clear", &json!({})).unwrap();
    assert_eq!((cleared["state"].as_str(), cleared["has_token"].as_bool()), (Some("off"), Some(false)));
    assert!(e.execute("tunnel.start", &json!({"own": true})).is_err());

    // On another computer (a new keychain), the project knows a token was saved but can't
    // find it: it says so instead of quietly making a temporary link.
    e.execute("tunnel.token.set", &json!({"token": token})).unwrap();
    drop(e);
    let e = Engine::open(&project).unwrap();
    e.use_memory_secrets();
    e.use_cloudflared(Some(fake));
    assert_eq!(e.execute("tunnel.status", &json!({})).unwrap()["has_token"], true);
    let err = e.execute("tunnel.start", &json!({})).unwrap_err().to_string();
    assert!(err.contains("can't find the token"), "{err}");
}

/// A made-up token against the real cloudflared and the real Cloudflare: refused, and
/// Porchly says so instead of connecting forever. Needs the internet
/// (`PORCHLY_TEST_CLOUDFLARED`).
#[test]
fn cloudflare_refusing_a_token_is_reported() {
    let Some(program) = std::env::var_os("PORCHLY_TEST_CLOUDFLARED").map(std::path::PathBuf::from) else {
        eprintln!("skipped: set PORCHLY_TEST_CLOUDFLARED to run the refused-token test");
        return;
    };
    let token = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMTExMTExMTEtMjIyMi0zMzMzLTQ0NDQtNTU1NTU1NTU1NTU1IiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQzF6WldOeVpYUT0ifQ==";
    let dir = tempfile::tempdir().unwrap();
    let e = Engine::open(dir.path().join("p.db")).unwrap();
    e.use_memory_secrets();
    e.use_cloudflared(Some(program));
    e.execute("tunnel.token.set", &json!({"token": token})).unwrap();
    e.execute("tunnel.start", &json!({})).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        let s = e.execute("tunnel.status", &json!({})).unwrap();
        if s["state"] == "failed" {
            break s;
        }
        assert_ne!(s["state"], "on", "a made-up token can't connect: {s}");
        assert!(Instant::now() < deadline, "still {s} after a minute");
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(status["error"].as_str().unwrap().contains("didn't accept the token"), "{status}");
    let log = std::fs::read_to_string(dir.path().join("logs/porch-light.log")).unwrap();
    assert!(!log.contains(token), "the token never reaches the log");
    e.execute("tunnel.stop", &json!({})).unwrap();
}
