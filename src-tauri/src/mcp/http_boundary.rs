//! HTTP-only admission guards. Application lifetime stays in lifecycle.rs.
use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{Method, StatusCode},
    middleware::Next,
    response::Response,
};
use tokio_util::sync::CancellationToken;

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const BODY_READ_TIMEOUT: Duration = Duration::from_secs(10);

// Listener shutdown alone cannot revoke requests on accepted keep-alive sockets.
// Gate every HTTP request, with cancellation outside the body-read deadline.
pub(super) async fn reject_cancelled(
    State(cancel): State<CancellationToken>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if cancel.is_cancelled() {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(StatusCode::SERVICE_UNAVAILABLE),
        response = next.run(request) => Ok(response),
    }
}

pub(super) async fn limit_body(request: Request, next: Next) -> Result<Response, StatusCode> {
    if request.method() != Method::POST {
        return Ok(next.run(request).await);
    }
    let (parts, body) = request.into_parts();
    let body = tokio::time::timeout(BODY_READ_TIMEOUT, to_bytes(body, MAX_REQUEST_BYTES))
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    Ok(next.run(Request::from_parts(parts, Body::from(body))).await)
}

#[cfg(test)]
#[path = "http_boundary_tests.rs"]
mod tests;
