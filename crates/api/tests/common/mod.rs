#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, dead_code)]

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use porchly_core::{FieldKind, Ident};
use porchly_schema::FieldDef;
use porchly_store::{Shared, Store, lock};
use serde_json::Value;
use tower::ServiceExt;

pub struct App {
    pub _dir: tempfile::TempDir,
    pub store: Shared,
    pub router: Router,
    pub read_key: String,
    pub write_key: String,
}

/// A project with a `recipes` table (name: required text, minutes: integer,
/// vegetarian: boolean), one read-only key and one read-write key.
pub fn app() -> App {
    app_with(porchly_api::router)
}

pub fn app_with(make_router: impl FnOnce(Shared) -> Router) -> App {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("p.db")).unwrap();
    let f = |n: &str, kind, required| FieldDef { name: Ident::new(n).unwrap(), kind, required, links_to: None };
    store
        .create_table(
            &Ident::new("recipes").unwrap(),
            vec![
                f("name", FieldKind::Text, true),
                f("minutes", FieldKind::Integer, false),
                f("vegetarian", FieldKind::Boolean, false),
            ],
        )
        .unwrap();
    let (_, read_key) = store.create_key("reader", false).unwrap();
    let (_, write_key) = store.create_key("writer", true).unwrap();
    let store = store.into_shared();
    let router = make_router(store.clone());
    App { _dir: dir, store, router, read_key, write_key }
}

impl App {
    pub async fn send(&self, method: Method, uri: &str, key: Option<&str>, body: Option<&str>) -> (StatusCode, Value) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(k) = key {
            req = req.header("authorization", format!("Bearer {k}"));
        }
        if body.is_some() {
            req = req.header("content-type", "application/json");
        }
        let req = req.body(body.map(|b| Body::from(b.to_owned())).unwrap_or_else(Body::empty)).unwrap();
        let res = self.router.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
        (status, value)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let key = self.read_key.clone();
        self.send(Method::GET, uri, Some(&key), None).await
    }

    pub async fn post(&self, uri: &str, body: &str) -> (StatusCode, Value) {
        let key = self.write_key.clone();
        self.send(Method::POST, uri, Some(&key), Some(body)).await
    }

    pub fn row_count(&self) -> u64 {
        let s = lock(&self.store);
        s.list_rows("recipes", &Default::default()).unwrap().total
    }
}
