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
