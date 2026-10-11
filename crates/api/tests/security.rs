//! Safe-by-default regression tests. Every security fix adds a test here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use axum::http::{Method, StatusCode};
use common::{app, app_with};
use porchly_core::limits;
use porchly_store::lock;

#[tokio::test]
async fn every_api_route_needs_a_key() {
    let a = app();
    for (method, uri) in [
        (Method::GET, "/api"),
        (Method::GET, "/api/recipes"),
        (Method::GET, "/api/recipes/1"),
        (Method::POST, "/api/recipes"),
        (Method::PATCH, "/api/recipes/1"),
        (Method::DELETE, "/api/recipes/1"),
    ] {
        let (status, body) = a.send(method.clone(), uri, None, Some("{}")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
        assert_eq!(body["error"]["code"], "unauthorized");
    }
}

#[tokio::test]
async fn bad_keys_are_refused() {
    let a = app();
    let truncated = a.read_key[..20].to_owned();
    let long = "pk_".to_owned() + &"a".repeat(5000);
    for key in ["", "pk_", "nonsense", truncated.as_str(), long.as_str()] {
        let (status, _) = a.send(Method::GET, "/api/recipes", Some(key), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "key {key:.30}");
    }
}

#[tokio::test]
async fn only_bearer_scheme_is_accepted() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    let a = app();
    for value in [format!("Basic {}", a.read_key), a.read_key.clone(), format!("Bearer{}", a.read_key)] {
        let req = Request::get("/api/recipes").header("authorization", value.as_str()).body(Body::empty()).unwrap();
        let res = a.router.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{value:.12}");
    }
    let req = Request::get("/api/recipes")
        .header("authorization", format!("bearer {}", a.read_key))
        .body(Body::empty())
        .unwrap();
    assert_eq!(a.router.clone().oneshot(req).await.unwrap().status(), StatusCode::OK, "scheme is case-insensitive");
}

#[tokio::test]
async fn read_keys_cannot_write() {
    let a = app();
    let key = a.read_key.clone();
    for (method, uri) in
        [(Method::POST, "/api/recipes"), (Method::PATCH, "/api/recipes/1"), (Method::DELETE, "/api/recipes/1")]
    {
        let (status, body) = a.send(method.clone(), uri, Some(&key), Some(r#"{"name":"x"}"#)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
        assert_eq!(body["error"]["code"], "forbidden");
    }
    assert_eq!(a.row_count(), 0);
}

#[tokio::test]
async fn revoked_keys_stop_working() {
    let a = app();
    assert_eq!(a.get("/api/recipes").await.0, StatusCode::OK);
    {
        let mut s = lock(&a.store);
        let id = s.list_keys().unwrap().into_iter().find(|k| k.name == "reader").unwrap().id;
        s.revoke_key(id).unwrap();
    }
    assert_eq!(a.get("/api/recipes").await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn porchly_and_sqlite_tables_are_unreachable() {
    let a = app();
    for uri in [
        "/api/_porchly_keys",
        "/api/_porchly_meta",
        "/api/_porchly_tables/1",
        "/api/sqlite_master",
        "/api/sqlite_sequence",
        "/api/%5Fporchly_keys",
        "/api/RECIPES",
    ] {
        let (status, body) = a.get(uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert!(!body.to_string().contains("hash"), "{uri}");
    }
}

#[tokio::test]
async fn injection_through_names_and_filters_fails_safely() {
    let a = app();
    a.post("/api/recipes", r#"{"name":"Soup"}"#).await;
    for uri in [
        "/api/recipes%22%3B%20DROP%20TABLE%20recipes%3B--",
        "/api/recipes?name%22%20OR%201%3D1--=x",
        "/api/recipes?sort=name%3B%20DROP%20TABLE%20recipes",
        "/api/recipes?minutes=1%20OR%201%3D1",
        "/api/recipes/1%20OR%201%3D1",
    ] {
        let (status, _) = a.get(uri).await;
        assert!(status == StatusCode::BAD_REQUEST || status == StatusCode::NOT_FOUND, "{uri}: {status}");
    }
    let (status, page) = a.get("/api/recipes?name=%27%20OR%20%271%27%3D%271").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 0, "SQL in a value is just a value");
    assert_eq!(a.row_count(), 1, "the table is intact");
}

#[tokio::test]
async fn system_fields_cannot_be_written() {
    let a = app();
    let (status, _) = a.post("/api/recipes", r#"{"name":"x","id":999}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = a.post("/api/recipes", r#"{"name":"x","created_at":"2000-01-01"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oversized_requests_are_refused() {
    let a = app();
    let big = format!(r#"{{"name":"{}"}}"#, "x".repeat(limits::MAX_BODY_BYTES + 10));
    let (status, body) = a.post("/api/recipes", &big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"]["code"], "too_large");

    let long_query = format!("/api/recipes?name={}", "x".repeat(limits::MAX_QUERY_BYTES + 1));
    assert_eq!(a.get(&long_query).await.0, StatusCode::PAYLOAD_TOO_LARGE);

    let filters: Vec<String> = (0..limits::MAX_FILTERS + 1).map(|_| "minutes=1".to_owned()).collect();
    let uri = format!("/api/recipes?{}", filters.join("&"));
    assert_eq!(a.get(&uri).await.0, StatusCode::PAYLOAD_TOO_LARGE);

    let (_, page) = a.get("/api/recipes?limit=999999").await;
    assert_eq!(page["limit"], limits::MAX_PAGE);
}

#[tokio::test]
async fn deeply_nested_json_is_refused() {
    let a = app();
    let deep = "[".repeat(10_000) + &"]".repeat(10_000);
    let (status, _) = a.post("/api/recipes", &format!(r#"{{"name":{deep}}}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rate_limit_applies_per_key() {
    let a = app_with(|store| porchly_api::router_with_rate_limit(store, 0.0001, 2.0));
    assert_eq!(a.get("/api/recipes").await.0, StatusCode::OK);
    assert_eq!(a.get("/api/recipes").await.0, StatusCode::OK);
    let (status, body) = a.get("/api/recipes").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    // Another key has its own allowance.
    let key = a.write_key.clone();
    assert_eq!(a.send(Method::GET, "/api/recipes", Some(&key), None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn responses_carry_protective_headers() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    let a = app();
    let res = a.router.clone().oneshot(Request::get("/health").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert_eq!(res.headers()["cache-control"], "no-store");
    let res = a.router.clone().oneshot(Request::get("/api").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.headers()["www-authenticate"], "Bearer");
    assert!(res.headers().get("access-control-allow-origin").is_none(), "no CORS by default");
}

#[tokio::test]
async fn filter_operators_cannot_inject_sql_or_widen_matches() {
    let a = app();
    a.post("/api/recipes", r#"{"name":"Soup"}"#).await;
    a.post("/api/recipes", r#"{"name":"Stew"}"#).await;
    for uri in [
        "/api/recipes?name=contains.%27%20OR%20%271%27%3D%271",
        "/api/recipes?name=in.x%27)%20OR%201%3D1%20--,y",
        "/api/recipes?name=starts.%25",
        "/api/recipes?name=contains._",
    ] {
        let (status, page) = a.get(uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(page["total"], 0, "{uri} matched rows it shouldn't");
    }
    for uri in ["/api/recipes?minutes=lt.1%20OR%201%3D1", "/api/recipes?minutes=in.1,2);DROP%20TABLE%20recipes;--"] {
        assert_eq!(a.get(uri).await.0, StatusCode::BAD_REQUEST, "{uri}");
    }
    let many = vec!["1"; limits::MAX_IN_VALUES + 1].join(",");
    assert_eq!(a.get(&format!("/api/recipes?minutes=in.{many}")).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(a.row_count(), 2, "the table is intact");
}

/// A stand-in for the functions runner: answers with what it was given.
struct Echo;

impl porchly_api::Functions for Echo {
    fn access(&self, name: &str) -> porchly_core::Result<porchly_api::FnAccess> {
        match name {
            "private" => Ok(porchly_api::FnAccess::Key),
            "hook" => Ok(porchly_api::FnAccess::Public),
            _ => Err(porchly_core::Error::not_found("There's no such function.")),
        }
    }

    fn list(&self) -> Vec<porchly_api::FnSummary> {
        Vec::new()
    }

    fn call(
        &self,
        call: porchly_api::FnCall,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = axum::response::Response> + Send + 'static>> {
        let body = serde_json::json!({
            "name": call.name,
            "path": call.path_and_query,
            "key": call.caller.key_name,
            "write": call.caller.can_write,
        });
        Box::pin(async move { axum::response::IntoResponse::into_response(axum::Json(body)) })
    }
}

const INTERNAL: &str = "pf_internal_token_for_tests_123456";

fn with_functions(store: porchly_store::Shared) -> axum::Router {
    let options = porchly_api::Options {
        functions: Some(std::sync::Arc::new(Echo)),
        internal_token: Some(INTERNAL.to_owned()),
        public: None,
        limits: None,
        stats: None,
    };
    porchly_api::router_with(store, options)
}

#[tokio::test]
async fn functions_need_a_key_unless_set_to_anyone() {
    let a = app_with(with_functions);
    let (status, _) = a.send(Method::POST, "/api/fn/private", None, Some("{}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = a.send(Method::POST, "/api/fn/private", Some("pk_wrong"), Some("{}")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, body) = a.get("/api/fn/private/orders/7?x=1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((body["path"].as_str(), body["key"].as_str()), (Some("/orders/7?x=1"), Some("reader")));
    assert_eq!(body["write"], false);

    let (status, body) = a.send(Method::POST, "/api/fn/hook", None, Some("{}")).await;
    assert_eq!(status, StatusCode::OK, "anyone may call a public function");
    assert_eq!(body["key"], serde_json::Value::Null);

    let (status, _) = a.get("/api/fn/missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // Without functions configured, there are none.
    let (status, _) = app().get("/api/fn/private").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn oversized_function_requests_are_refused_up_front() {
    let a = app_with(with_functions);
    let req = axum::http::Request::builder()
        .method(Method::POST)
        .uri("/api/fn/hook")
        .header("content-length", (limits::MAX_FUNCTION_BODY_BYTES + 1).to_string())
        .body(axum::body::Body::empty())
        .unwrap();
    let res = tower::ServiceExt::oneshot(a.router.clone(), req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_internal_key_works_only_when_configured() {
    let a = app_with(with_functions);
    let (status, _) = a.send(Method::POST, "/api/recipes", Some(INTERNAL), Some(r#"{"name":"Soup"}"#)).await;
    assert_eq!(status, StatusCode::CREATED, "functions may change data");
    // Not rate limited: far more than a key's burst.
    for _ in 0..100 {
        assert_eq!(a.send(Method::GET, "/api/recipes", Some(INTERNAL), None).await.0, StatusCode::OK);
    }
    assert_eq!(app().send(Method::GET, "/api/recipes", Some(INTERNAL), None).await.0, StatusCode::UNAUTHORIZED);
    // A short internal key is never accepted, even if configured.
    let weak = app_with(|s| {
        porchly_api::router_with(s, porchly_api::Options { internal_token: Some("short".into()), ..Default::default() })
    });
    assert_eq!(weak.send(Method::GET, "/api/recipes", Some("short"), None).await.0, StatusCode::UNAUTHORIZED);
    let empty = app_with(|s| {
        porchly_api::router_with(s, porchly_api::Options { internal_token: Some(String::new()), ..Default::default() })
    });
    assert_eq!(empty.send(Method::GET, "/api/recipes", Some(""), None).await.0, StatusCode::UNAUTHORIZED);
}

fn public_link(
    allow: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> impl FnOnce(porchly_store::Shared) -> axum::Router {
    move |store| {
        let options = porchly_api::Options {
            functions: Some(std::sync::Arc::new(Echo)),
            internal_token: Some(INTERNAL.to_owned()),
            public: Some(allow),
            limits: None,
            stats: None,
        };
        porchly_api::router_with(store, options)
    }
}

#[tokio::test]
async fn the_public_link_is_read_only_until_changes_are_allowed() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let allow = std::sync::Arc::new(AtomicBool::new(false));
    let a = app_with(public_link(allow.clone()));
    let (status, _) = a.get("/api/recipes").await;
    assert_eq!(status, StatusCode::OK, "reading works");
    let (status, body) = a.post("/api/recipes", r#"{"name":"Soup"}"#).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "even a key that may write");
    assert!(body["error"]["message"].as_str().unwrap().contains("public link"));
    let write_key = a.write_key.clone();
    for (method, uri) in [(Method::PATCH, "/api/recipes/1"), (Method::DELETE, "/api/recipes/1")] {
        assert_eq!(a.send(method, uri, Some(&write_key), Some("{}")).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(a.row_count(), 0);
    // Functions are told the caller may not make changes.
    let (_, body) = a.send(Method::POST, "/api/fn/private", Some(&write_key), Some("{}")).await;
    assert_eq!(body["write"], false);

    // The owner ticks "Allow changes": takes effect at once.
    allow.store(true, Ordering::Relaxed);
    let (status, _) = a.post("/api/recipes", r#"{"name":"Soup"}"#).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, body) = a.send(Method::POST, "/api/fn/private", Some(&write_key), Some("{}")).await;
    assert_eq!(body["write"], true);
    // Read-only keys stay read-only.
    let read_key = a.read_key.clone();
    assert_eq!(
        a.send(Method::POST, "/api/recipes", Some(&read_key), Some(r#"{"name":"x"}"#)).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn the_internal_key_never_works_through_the_public_link() {
    let allow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let a = app_with(public_link(allow));
    assert_eq!(a.send(Method::GET, "/api/recipes", Some(INTERNAL), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(a.send(Method::GET, "/api/recipes", None, None).await.0, StatusCode::UNAUTHORIZED, "keys still needed");
}

#[tokio::test]
async fn the_request_limit_follows_settings_at_once() {
    let limits = std::sync::Arc::new(porchly_api::Limits::new(5));
    let shared = limits.clone();
    let a = app_with(move |store| {
        porchly_api::router_with(store, porchly_api::Options { limits: Some(shared), ..Default::default() })
    });
    let mut ok = 0;
    for _ in 0..40 {
        if a.get("/api/recipes").await.0 == StatusCode::OK {
            ok += 1;
        }
    }
    assert!((15..20).contains(&ok), "Gentle: a burst of about 15, got {ok}");
    let (status, body) = a.get("/api/recipes").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"]["code"], "rate_limited");
    limits.set_per_second(0);
    assert_eq!(limits.per_second(), 1, "the limit can't be turned off");
}

#[tokio::test]
async fn bulk_changes_need_a_write_key_and_a_condition() {
    let a = app();
    a.post("/api/recipes", r#"[{"name":"Soup"},{"name":"Salad"}]"#).await;
    let (read, write) = (a.read_key.clone(), a.write_key.clone());

    // Read-only keys and missing keys can't change or delete anything.
    for key in [Some(read.as_str()), None] {
        let expected = if key.is_some() { StatusCode::FORBIDDEN } else { StatusCode::UNAUTHORIZED };
        assert_eq!(a.send(Method::POST, "/api/recipes", key, Some(r#"[{"name":"x"}]"#)).await.0, expected);
        assert_eq!(a.send(Method::PATCH, "/api/recipes?id=gte.1", key, Some(r#"{"name":"x"}"#)).await.0, expected);
        assert_eq!(a.send(Method::DELETE, "/api/recipes?id=gte.1", key, None).await.0, expected);
    }

    // A forgotten condition never touches the whole table.
    let (status, body) = a.send(Method::DELETE, "/api/recipes", Some(&write), None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().contains("Say which rows to delete"), "{body}");
    let (status, _) = a.send(Method::PATCH, "/api/recipes", Some(&write), Some(r#"{"name":"x"}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Paging would suggest only some rows are affected: refused.
    assert_eq!(a.send(Method::DELETE, "/api/recipes?limit=1", Some(&write), None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        a.send(Method::DELETE, "/api/recipes?id=gte.1&limit=1", Some(&write), None).await.0,
        StatusCode::BAD_REQUEST
    );
    // Unknown fields and hostile conditions are refused, not ignored.
    assert_eq!(a.send(Method::DELETE, "/api/recipes?nope=1", Some(&write), None).await.0, StatusCode::BAD_REQUEST);
    let hostile = "/api/recipes?name=x%27%20OR%201%3D1%20--";
    assert_eq!(
        a.send(Method::DELETE, hostile, Some(&write), None).await.1["deleted"],
        0,
        "bound as a value, never SQL"
    );
    assert_eq!(a.row_count(), 2, "nothing was deleted or changed");

    // Too many rows in one request.
    let many = format!("[{}]", vec![r#"{"name":"x"}"#; limits::MAX_BULK_ROWS + 1].join(","));
    assert_eq!(a.post("/api/recipes", &many).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(a.row_count(), 2);
}

#[tokio::test]
async fn bulk_changes_are_refused_through_a_read_only_public_link() {
    let allow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a = app_with(public_link(allow));
    let write = a.write_key.clone();
    assert_eq!(
        a.send(Method::POST, "/api/recipes", Some(&write), Some(r#"[{"name":"x"}]"#)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        a.send(Method::PATCH, "/api/recipes?id=gte.1", Some(&write), Some(r#"{"name":"x"}"#)).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(a.send(Method::DELETE, "/api/recipes?id=gte.1", Some(&write), None).await.0, StatusCode::FORBIDDEN);
}

/// Sends one MCP message; returns the status and the answer.
async fn mcp(a: &common::App, key: Option<&str>, message: serde_json::Value) -> (StatusCode, serde_json::Value) {
    a.send(Method::POST, "/mcp", key, Some(&message.to_string())).await
}

fn tool_names(answer: &serde_json::Value) -> Vec<String> {
    answer["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test]
async fn ai_assistants_get_only_what_their_key_allows() {
    use serde_json::json;
    let a = app();
    a.post("/api/recipes", r#"[{"name":"Soup"},{"name":"Salad"}]"#).await;
    let (read, write) = (a.read_key.clone(), a.write_key.clone());
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let delete = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "delete_rows", "arguments": {"table": "recipes", "ids": [1]}}});

    // No key, or a wrong one: nothing at all, not even the list of tools.
    assert_eq!(mcp(&a, None, list.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(mcp(&a, Some("pk_wrong"), list.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(a.send(Method::GET, "/mcp", Some(&read), None).await.0, StatusCode::METHOD_NOT_ALLOWED);

    // A read-only key reads, and isn't even offered the tools that change things.
    let (status, answer) = mcp(&a, Some(&read), list.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tool_names(&answer), ["list_tables", "list_rows", "get_row"]);
    let (_, answer) = mcp(&a, Some(&read), delete.clone()).await;
    assert_eq!(answer["result"]["isError"], true, "calling it anyway is refused");
    assert_eq!(a.row_count(), 2);

    // A key that may make changes can; making tables stays off until the owner allows it.
    let (_, answer) = mcp(&a, Some(&write), list.clone()).await;
    let names = tool_names(&answer);
    assert!(names.contains(&"delete_rows".to_owned()) && !names.contains(&"create_table".to_owned()));
    let (_, answer) = mcp(&a, Some(&write), delete.clone()).await;
    assert_eq!(answer["result"]["isError"], false);
    assert_eq!(a.row_count(), 1);
    lock(&a.store).set_setting(porchly_api::mcp::TABLES_SETTING, Some("on")).unwrap();
    let (_, answer) = mcp(&a, Some(&write), list.clone()).await;
    assert!(tool_names(&answer).contains(&"create_table".to_owned()));
    let (_, answer) = mcp(&a, Some(&read), list).await;
    assert_eq!(tool_names(&answer).len(), 3, "never for a read-only key");

    // Notifications are accepted without an answer; junk is refused politely.
    let (status, body) = mcp(&a, Some(&read), json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!((status, body), (StatusCode::ACCEPTED, serde_json::Value::Null));
    assert_eq!(a.send(Method::POST, "/mcp", Some(&read), Some("{not json")).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ai_assistants_can_only_read_through_a_read_only_public_link() {
    use serde_json::json;
    let allow = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a = app_with(public_link(allow.clone()));
    lock(&a.store).set_setting(porchly_api::mcp::TABLES_SETTING, Some("on")).unwrap();
    let write = a.write_key.clone();
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let add = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "add_rows", "arguments": {"table": "recipes", "rows": [{"name": "x"}]}}});
    let (_, answer) = mcp(&a, Some(&write), list.clone()).await;
    assert_eq!(tool_names(&answer), ["list_tables", "list_rows", "get_row"], "even a key that may make changes");
    let (_, answer) = mcp(&a, Some(&write), add.clone()).await;
    assert_eq!(answer["result"]["isError"], true);
    assert_eq!(a.row_count(), 0);
    assert_eq!(
        mcp(&a, Some(INTERNAL), list.clone()).await.0,
        StatusCode::UNAUTHORIZED,
        "the functions' key doesn't work here"
    );

    allow.store(true, std::sync::atomic::Ordering::Relaxed);
    let (_, answer) = mcp(&a, Some(&write), add).await;
    assert_eq!(answer["result"]["isError"], false, "once the owner allows changes through the link");
    assert_eq!(a.row_count(), 1);
}
