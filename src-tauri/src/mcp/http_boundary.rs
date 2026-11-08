//! HTTP-only admission guards. Application lifetime stays in lifecycle.rs.
use std::time::Duration;

use axum::{
    body::{to_bytes, Body},
    extract::{MatchedPath, Request, State},
    http::{request::Parts, uri::Authority, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use rmcp::model::{ClientJsonRpcMessage, ClientRequest};
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

pub(super) async fn limit_body(
    State(port): State<u16>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    if request.method() != Method::POST {
        return Ok(next.run(request).await);
    }
    let (parts, body) = request.into_parts();
    let body = tokio::time::timeout(BODY_READ_TIMEOUT, to_bytes(body, MAX_REQUEST_BYTES))
        .await
        .map_err(|_| StatusCode::REQUEST_TIMEOUT)?
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    // rmcp 1.7 allocates a stateful session before checking that the first
    // message is initialize. Rejected first messages then leave an unreachable
    // handle in its session map. Reject those messages before allocation, using
    // the same typed parser and absent/non-text session-header semantics.
    // Invalid JSON is left to rmcp, which rejects it before creating a session.
    let json_post = parts.headers.get("content-type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/json"));
    let accepts_mcp = parts.headers.get("accept")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json") && value.contains("text/event-stream"));
    // Leave explicit routes, DNS rebinding checks and HTTP negotiation errors
    // to their original handlers. This guard runs before rmcp's HTTP checks.
    if json_post && accepts_mcp
        && parts.extensions.get::<MatchedPath>().is_none()
        && uses_allowed_loopback_authority(&parts, port)
        && parts.headers.get("mcp-session-id").and_then(|value| value.to_str().ok()).is_none()
    {
        if let Ok(message) = serde_json::from_slice::<ClientJsonRpcMessage>(&body) {
            if !matches!(message, ClientJsonRpcMessage::Request(request)
                if matches!(request.request, ClientRequest::InitializeRequest(_)))
            {
                return Ok(Response::builder()
                    .status(StatusCode::UNPROCESSABLE_ENTITY)
                    .body(Body::from("Unexpected message, expect initialize request"))
                    .expect("valid admission response"));
            }
        }
    }
    Ok(next.run(Request::from_parts(parts, Body::from(body))).await)
}

// Match the configured loopback authorities and rmcp 1.7's URI normalization.
// This does not grant access: everything not rejected here still passes
// through rmcp's authoritative Host/Origin checks. Keeping their order also
// preserves malformed-header 400 and untrusted-origin 403 responses.
fn uses_allowed_loopback_authority(parts: &Parts, port: u16) -> bool {
    fn allowed(authority: &Authority, port: u16) -> bool {
        let host = authority.host().trim_matches('[').trim_matches(']');
        (host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1")
            && authority.port_u16() == Some(port)
    }

    let host = match parts.headers.get("host") {
        Some(value) => value.to_str().ok().and_then(|value| value.parse::<Authority>().ok()),
        None => parts.uri.authority().cloned(),
    };
    if !host.as_ref().is_some_and(|host| allowed(host, port)) {
        return false;
    }
    match parts.headers.get("origin") {
        None => true,
        Some(value) => value.to_str().ok()
            .and_then(|value| value.trim().parse::<axum::http::Uri>().ok())
            .is_some_and(|origin| {
                origin.scheme_str().is_some_and(|scheme| scheme.eq_ignore_ascii_case("http"))
                    && origin.authority().is_some_and(|authority| allowed(authority, port))
            }),
    }
}

#[cfg(test)]
#[path = "http_boundary_tests.rs"]
mod tests;
