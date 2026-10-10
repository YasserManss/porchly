//! The route to functions: `/api/fn/{name}/…`. The API checks who may call (a key, or anyone
//! for functions set to "no key") and hands the request to [`Functions`], which runs the code.
//! What the function does is up to its owner; Porchly doesn't control it.

use std::future::Future;
use std::pin::Pin;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use porchly_core::{Error, limits};
use serde::Serialize;

use crate::{Access, ApiError, AppState, authorize};

/// Who may call a function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FnAccess {
    /// Any valid key (the default).
    #[default]
    Key,
    /// Anyone, without a key (for webhooks).
    Public,
}

/// A function as the docs list it.
#[derive(Debug, Clone, Serialize)]
pub struct FnSummary {
    pub name: String,
    pub access: FnAccess,
}

/// Who is calling: the key's name and whether it may make changes. `None` for a call
/// without a key to a function anyone may use.
#[derive(Debug, Clone, Default)]
pub struct FnCaller {
    pub key_name: Option<String>,
    pub can_write: bool,
}

/// One call to forward.
pub struct FnCall {
    pub name: String,
    /// What the function sees as its address: `/orders/7?x=1` for `/api/fn/shop/orders/7?x=1`.
    pub path_and_query: String,
    pub request: Request,
    pub caller: FnCaller,
}

/// Runs functions. Implemented outside the API crate (it starts programs), so the API only
/// knows this interface.
pub trait Functions: Send + Sync + 'static {
    /// Who may call `name`. `Err(NotFound)` when there's no such function or they're off.
    fn access(&self, name: &str) -> porchly_core::Result<FnAccess>;
    /// Every function, for the docs.
    fn list(&self) -> Vec<FnSummary>;
    /// Runs the function with the request and returns its response.
    fn call(&self, call: FnCall) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>;
}

pub(crate) async fn route(State(s): State<AppState>, request: Request) -> Response {
    match forward(s, request).await {
        Ok(res) => res,
        Err(e) => e.into_response(),
    }
}

async fn forward(s: AppState, request: Request) -> Result<Response, ApiError> {
    let Some(functions) = s.functions.clone() else {
        return Err(Error::not_found("This project has no functions.").into());
    };
    let path = request.uri().path().strip_prefix("/api/fn/").unwrap_or_default();
    let (name, rest) = path.split_once('/').unwrap_or((path, ""));
    let name = name.to_owned();
    let caller = match functions.access(&name)? {
        FnAccess::Key => {
            let key = authorize(&s, request.headers(), Access::Read).await?;
            FnCaller { key_name: Some(key.name), can_write: key.can_write && crate::changes_allowed(&s) }
        }
        FnAccess::Public => FnCaller::default(),
    };
    let declared =
        request.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|n| n > limits::MAX_FUNCTION_BODY_BYTES as u64) {
        return Err(too_large().into());
    }
    let path_and_query = match request.uri().query() {
        Some(q) => format!("/{rest}?{q}"),
        None => format!("/{rest}"),
    };
    let request = request.map(|body| Body::new(http_body_util::Limited::new(body, limits::MAX_FUNCTION_BODY_BYTES)));
    Ok(functions.call(FnCall { name, path_and_query, request, caller }).await)
}

fn too_large() -> Error {
    Error::TooLarge(format!(
        "The request is too big for a function (the limit is {} MB).",
        limits::MAX_FUNCTION_BODY_BYTES / (1024 * 1024)
    ))
}
