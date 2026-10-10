use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use porchly_core::Error;
use serde_json::json;

/// An error as the API returns it: `{"error": {"code": "...", "message": "..."}}`.
/// Storage and internal errors are logged and replaced by a generic message.
#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    pub(crate) fn rate_limited() -> Self {
        ApiError {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited",
            message: "Too many requests with this key. Please slow down and try again in a moment.".into(),
        }
    }
}

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        let status = match &e {
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::NewerFormat { .. } | Error::Storage(_) | Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let message = if e.is_public() {
            e.to_string()
        } else {
            tracing::error!("internal error: {e}");
            "Something went wrong on this Porchly. The owner can find details in its log.".to_owned()
        };
        ApiError { status, code: e.code(), message }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut res = (self.status, axum::Json(json!({ "error": { "code": self.code, "message": self.message } })))
            .into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            res.headers_mut()
                .insert(axum::http::header::WWW_AUTHENTICATE, axum::http::HeaderValue::from_static("Bearer"));
        }
        res
    }
}
