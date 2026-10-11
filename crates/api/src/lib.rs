//! The HTTP API: every table in the project becomes a set of REST routes.
//!
//! Safe by default: every `/api` route needs a key, read-only keys can't change anything,
//! requests are size-capped and rate-limited, and internal error details never leave the
//! server. See `docs/api.md`.

mod bind;
pub mod docs;
mod error;
mod functions;
mod limiter;
pub mod mcp;
mod openapi;
mod stats;

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, RawQuery, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use porchly_core::{Error, Result, limits};
use porchly_store::{KeyInfo, Shared, Store, lock};
use serde_json::{Value, json};

pub use bind::{PORT_TRIES, bind};
pub use error::ApiError;
pub use functions::{FnAccess, FnCall, FnCaller, FnSummary, Functions};
use limiter::RateLimiter;
pub use limiter::{DEFAULT_PER_SECOND, Limits};
pub use stats::Stats;

/// The port Porchly serves on unless told otherwise. Above 1024, so it never needs
/// administrator rights, and uncommon enough to rarely clash with other apps.
pub const DEFAULT_PORT: u16 = 11911;

#[derive(Clone)]
struct AppState {
    store: Shared,
    limiter: Arc<RateLimiter>,
    functions: Option<Arc<dyn Functions>>,
    internal_token: Option<Arc<str>>,
    /// Set when this server is behind the public link; true while changes are allowed.
    public: Option<Arc<AtomicBool>>,
    stats: Arc<Stats>,
}

/// What the API serves besides the tables.
#[derive(Clone, Default)]
pub struct Options {
    /// Runs functions at `/api/fn/{name}`. Without it, there are no functions.
    pub functions: Option<Arc<dyn Functions>>,
    /// A key that only Porchly's own functions get: may read and change everything, isn't
    /// rate limited, never stored. Made fresh each time Porchly starts.
    pub internal_token: Option<String>,
    /// Set for the server behind the public link (the porch light). Requests through it
    /// can't make changes unless the flag is true ("Allow changes" in the app), and the
    /// internal key isn't accepted.
    pub public: Option<Arc<AtomicBool>>,
    /// The rate limit, shared so Settings can change it while the server runs.
    /// Without it, the default (20 requests a second per key) applies.
    pub limits: Option<Arc<Limits>>,
    /// Usage numbers, shared so the app can show them (and so both servers count together).
    pub stats: Option<Arc<Stats>>,
}

/// The name the internal token shows up as (to functions, in `X-Porchly-Key-Name`).
pub const INTERNAL_KEY_NAME: &str = "Porchly functions";

/// The API router for a project. Mount it, or run it with [`serve`].
pub fn router(store: Shared) -> Router {
    router_with(store, Options::default())
}

/// The router with functions and the internal token.
pub fn router_with(store: Shared, options: Options) -> Router {
    let limiter = RateLimiter::shared(options.limits.clone().unwrap_or_default());
    router_with_limiter(store, limiter, options)
}

/// The router with a custom rate limit (requests per second and burst size per key).
pub fn router_with_rate_limit(store: Shared, per_second: f64, burst: f64) -> Router {
    router_with_limiter(store, RateLimiter::new(per_second, burst), Options::default())
}

fn router_with_limiter(store: Shared, limiter: RateLimiter, options: Options) -> Router {
    let state = AppState {
        store,
        limiter: Arc::new(limiter),
        functions: options.functions,
        // Functions run on this computer and use the local address, never the public link.
        internal_token: options.internal_token.filter(|t| t.len() >= 16 && options.public.is_none()).map(Arc::from),
        public: options.public,
        stats: options.stats.unwrap_or_default(),
    };
    Router::new()
        .route("/health", get(health))
        .route("/api", get(index))
        .route("/api/openapi.json", get(openapi_json))
        .route("/docs", get(docs_page))
        .route("/mcp", post(mcp_http))
        .route("/api/fn/{name}", any(functions::route))
        .route("/api/fn/{name}/{*rest}", any(functions::route))
        .route("/api/{table}", get(list).post(create).patch(update_many).delete(remove_many))
        .route("/api/{table}/{id}", get(get_one).patch(update).delete(remove))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(DefaultBodyLimit::max(limits::MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), log_and_harden))
        .with_state(state)
}

/// Serves the API on an already bound listener (see [`bind`]) until `shutdown` completes.
pub async fn serve(
    store: Shared,
    listener: std::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    serve_with(store, listener, Options::default(), shutdown).await
}

/// [`serve`], with functions and the internal token.
pub async fn serve_with(
    store: Shared,
    listener: std::net::TcpListener,
    options: Options,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    tracing::info!("Porchly API listening on http://{}", listener.local_addr()?);
    axum::serve(listener, router_with(store, options)).with_graceful_shutdown(shutdown).await
}

async fn health() -> Response {
    axum::Json(json!({ "ok": true })).into_response()
}

async fn index(State(s): State<AppState>, headers: HeaderMap) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Read).await?;
    let tables = blocking(&s, |store| store.list_tables()).await?;
    Ok(axum::Json(json!({ "tables": tables })).into_response())
}

async fn openapi_json(State(s): State<AppState>, headers: HeaderMap) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Read).await?;
    let tables = blocking(&s, |store| store.list_tables()).await?;
    Ok(axum::Json(openapi::spec(&tables, &function_list(&s))).into_response())
}

fn function_list(s: &AppState) -> Vec<FnSummary> {
    s.functions.as_ref().map(|f| f.list()).unwrap_or_default()
}

/// The readable guide to this API. Needs a key, like everything that reveals the tables.
async fn docs_page(State(s): State<AppState>, headers: HeaderMap) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Read).await?;
    let tables = blocking(&s, |store| store.list_tables()).await?;
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("127.0.0.1");
    // The public link is always https (Cloudflare adds it); this computer is plain http.
    let scheme = if s.public.is_some() { "https" } else { "http" };
    let page = docs::html(&tables, &function_list(&s), &format!("{scheme}://{host}"));
    Ok(([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response())
}

async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(table): Path<String>,
    RawQuery(query): RawQuery,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Read).await?;
    let query = query.unwrap_or_default();
    if query.len() > limits::MAX_QUERY_BYTES {
        return Err(Error::TooLarge("The query string is too long.".into()).into());
    }
    let page = blocking(&s, move |store| {
        let pairs: Vec<(String, String)> =
            form_urlencoded::parse(query.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let q = store.list_query(&table, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
        store.list_rows(&table, &q)
    })
    .await?;
    Ok(axum::Json(page).into_response())
}

async fn get_one(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((table, id)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Read).await?;
    let id = parse_id(&id)?;
    let query = query.unwrap_or_default();
    if query.len() > limits::MAX_QUERY_BYTES {
        return Err(Error::TooLarge("The query string is too long.".into()).into());
    }
    // A single row only takes `expand`.
    let mut expand = None;
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        if k != "expand" {
            return Err(Error::invalid(format!(
                "A single row only takes \"expand\", not \"{}\".",
                k.chars().take(40).collect::<String>()
            ))
            .into());
        }
        expand = Some(v.into_owned());
    }
    let row = blocking(&s, move |store| match expand {
        Some(e) => store.get_row_expanded(&table, id, &e),
        None => store.get_row(&table, id),
    })
    .await?;
    Ok(axum::Json(row).into_response())
}

async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(table): Path<String>,
    body: std::result::Result<Bytes, BytesRejection>,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Write).await?;
    match json_body(body)? {
        // A list of rows: all are added, or none.
        Value::Array(rows) => {
            let rows = rows
                .into_iter()
                .enumerate()
                .map(|(i, r)| match r {
                    Value::Object(map) => Ok(map),
                    _ => Err(Error::invalid(format!("Row {}: each row must be a JSON object.", i + 1))),
                })
                .collect::<Result<Vec<_>>>()?;
            let items = blocking(&s, move |store| store.insert_rows_returning(&table, &rows)).await?;
            Ok((StatusCode::CREATED, axum::Json(json!({ "count": items.len(), "items": items }))).into_response())
        }
        Value::Object(body) => {
            let row = blocking(&s, move |store| store.insert_row(&table, &body)).await?;
            Ok((StatusCode::CREATED, axum::Json(row)).into_response())
        }
        _ => Err(Error::invalid(
            "Send a JSON object for one row, like {\"name\": \"Pancakes\"}, or a list of objects for several.",
        )
        .into()),
    }
}

/// Changes every row matching the conditions in the query string.
async fn update_many(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(table): Path<String>,
    RawQuery(query): RawQuery,
    body: std::result::Result<Bytes, BytesRejection>,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Write).await?;
    let pairs = query_pairs(query)?;
    let body = json_object(body)?;
    let updated = blocking(&s, move |store| {
        let q = store.list_query(&table, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
        store.update_rows(&table, &q, &body)
    })
    .await?;
    Ok(axum::Json(json!({ "updated": updated })).into_response())
}

/// Deletes every row matching the conditions in the query string.
async fn remove_many(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(table): Path<String>,
    RawQuery(query): RawQuery,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Write).await?;
    let pairs = query_pairs(query)?;
    let deleted = blocking(&s, move |store| {
        let q = store.list_query(&table, pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
        store.delete_rows(&table, &q)
    })
    .await?;
    Ok(axum::Json(json!({ "deleted": deleted })).into_response())
}

/// The query string as key/value pairs, size-checked.
fn query_pairs(query: Option<String>) -> Result<Vec<(String, String)>> {
    let query = query.unwrap_or_default();
    if query.len() > limits::MAX_QUERY_BYTES {
        return Err(Error::TooLarge("The query string is too long.".into()));
    }
    Ok(form_urlencoded::parse(query.as_bytes()).map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
}

async fn update(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((table, id)): Path<(String, String)>,
    body: std::result::Result<Bytes, BytesRejection>,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Write).await?;
    let id = parse_id(&id)?;
    let body = json_object(body)?;
    let row = blocking(&s, move |store| store.update_row(&table, id, &body)).await?;
    Ok(axum::Json(row).into_response())
}

async fn remove(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((table, id)): Path<(String, String)>,
) -> std::result::Result<Response, ApiError> {
    authorize(&s, &headers, Access::Write).await?;
    let id = parse_id(&id)?;
    blocking(&s, move |store| store.delete_row(&table, id)).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// MCP for AI assistants: one JSON-RPC message in, one answer out. Same keys as the rest
/// of the API; what a key may do decides which tools it gets (see [`mcp`]).
async fn mcp_http(
    State(s): State<AppState>,
    headers: HeaderMap,
    body: std::result::Result<Bytes, BytesRejection>,
) -> std::result::Result<Response, ApiError> {
    let key = authorize(&s, &headers, Access::Read).await?;
    let can_write = key.can_write && changes_allowed(&s);
    let message = json_body(body)?;
    let answer = blocking(&s, move |store| {
        let caller = mcp::Caller::new(store, can_write);
        Ok(mcp::handle(store, caller, &message))
    })
    .await?;
    Ok(match answer {
        Some(answer) => axum::Json(answer).into_response(),
        // Notifications have no answer.
        None => StatusCode::ACCEPTED.into_response(),
    })
}

async fn not_found() -> ApiError {
    Error::not_found("There's nothing here. The API lives under /api.").into()
}

async fn method_not_allowed() -> Response {
    let body =
        json!({ "error": { "code": "method_not_allowed", "message": "This address doesn't accept that method." } });
    (StatusCode::METHOD_NOT_ALLOWED, axum::Json(body)).into_response()
}

#[derive(Clone, Copy, PartialEq)]
enum Access {
    Read,
    Write,
}

/// Checks the bearer key, its permission and its rate limit.
async fn authorize(s: &AppState, headers: &HeaderMap, access: Access) -> std::result::Result<KeyInfo, ApiError> {
    let presented = bearer(headers).ok_or(Error::Unauthorized)?;
    if let Some(internal) = &s.internal_token
        && constant_time_eq(presented.as_bytes(), internal.as_bytes())
    {
        s.stats.key_used(0, INTERNAL_KEY_NAME);
        return Ok(KeyInfo {
            id: 0,
            name: INTERNAL_KEY_NAME.to_owned(),
            prefix: String::new(),
            can_write: true,
            created_at: String::new(),
            revoked_at: None,
        });
    }
    let key = blocking(s, move |store| store.verify_key(&presented)).await?.ok_or(Error::Unauthorized)?;
    s.stats.key_used(key.id, &key.name);
    if !s.limiter.allow(key.id) {
        return Err(ApiError::rate_limited());
    }
    if access == Access::Write && !changes_allowed(s) {
        return Err(Error::Forbidden(
            "Changes through the public link are turned off. The owner can allow them in Porchly, under Share.".into(),
        )
        .into());
    }
    if access == Access::Write && !key.can_write {
        return Err(Error::Forbidden(format!(
            "The key \"{}\" can only read. Ask the owner for a key that can make changes.",
            key.name
        ))
        .into());
    }
    Ok(key)
}

/// False behind the public link while the owner hasn't allowed changes through it.
pub(crate) fn changes_allowed(s: &AppState) -> bool {
    s.public.as_ref().is_none_or(|allowed| allowed.load(Ordering::Relaxed))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim().to_owned())
}

/// Runs store work off the async threads. A panic inside is caught by the runtime and
/// reported as an internal error; the server keeps serving.
async fn blocking<T, F>(s: &AppState, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&mut Store) -> Result<T> + Send + 'static,
{
    let store = s.store.clone();
    tokio::task::spawn_blocking(move || f(&mut lock(&store)))
        .await
        .map_err(|e| Error::Internal(format!("request task failed: {e}")))?
}

fn parse_id(s: &str) -> Result<i64> {
    s.parse::<i64>()
        .map_err(|_| Error::not_found(format!("There's no row \"{}\".", s.chars().take(40).collect::<String>())))
}

fn json_object(body: std::result::Result<Bytes, BytesRejection>) -> Result<serde_json::Map<String, Value>> {
    match json_body(body)? {
        Value::Object(map) => Ok(map),
        _ => Err(Error::invalid("Send a JSON object, like {\"name\": \"Pancakes\"}.")),
    }
}

fn json_body(body: std::result::Result<Bytes, BytesRejection>) -> Result<Value> {
    let bytes = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            Error::TooLarge(format!("The request is too big (the limit is {} KB).", limits::MAX_BODY_BYTES / 1024))
        } else {
            Error::invalid("Couldn't read the request body.")
        }
    })?;
    serde_json::from_slice::<Value>(&bytes).map_err(|e| Error::invalid(format!("The body isn't valid JSON: {e}")))
}

/// Logs each request (method, path, status, time; never headers or query strings, which
/// may hold keys or personal data) and adds protective headers.
async fn log_and_harden(State(s): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().chars().take(200).collect::<String>();
    let started = Instant::now();
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    if path.starts_with("/api/fn/") {
        // A function decides its own headers; these are only the fallback.
        headers.entry(header::X_CONTENT_TYPE_OPTIONS).or_insert(HeaderValue::from_static("nosniff"));
        headers.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    } else {
        headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    s.stats.record(&method, &path, res.status().as_u16(), s.public.is_some());
    tracing::info!("{method} {path} {} {}ms", res.status().as_u16(), started.elapsed().as_millis());
    res
}
