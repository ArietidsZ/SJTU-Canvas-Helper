use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

use chrono::Utc;
use reqwest::{header::HeaderMap, Request, Response};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::model::{DebugHttpHeader, NetworkRequestLog};

const MAX_NETWORK_LOGS: usize = 1000;
// Private response metadata ties body diagnostics to the request that produced them.
#[derive(Clone, Debug)]
struct RequestLogId(String);

#[derive(Debug)]
pub struct NetworkDebugStore {
    enabled: AtomicBool,
    logs: RwLock<VecDeque<NetworkRequestLog>>,
}

impl NetworkDebugStore {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: AtomicBool::new(enabled),
            logs: RwLock::new(VecDeque::with_capacity(MAX_NETWORK_LOGS)),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub async fn list_logs(&self) -> Vec<NetworkRequestLog> {
        self.logs.read().await.iter().rev().cloned().collect()
    }

    pub async fn clear_logs(&self) {
        self.logs.write().await.clear();
    }

    pub fn response_id(&self, response: &Response) -> Option<String> {
        response.extensions().get::<RequestLogId>().map(|id| id.0.clone())
    }

    pub async fn capture_response_body(&self, request_id: Option<&str>, body: &[u8]) {
        let Some(request_id) = request_id else { return; };
        if !self.enabled.load(Ordering::Relaxed) { return; }
        let mut logs = self.logs.write().await;
        if !self.enabled.load(Ordering::Relaxed) { return; }
        if let Some(log) = logs.iter_mut().find(|log| log.id == request_id && log.response_body.is_none()) {
            log.response_body = Some(omitted_body(body.len()));
            log.response_body_truncated = !body.is_empty();
        }
    }

    pub async fn send(
        &self,
        client: &reqwest::Client,
        request: Request,
    ) -> std::result::Result<Response, reqwest::Error> {
        let log = self.enabled.load(Ordering::Relaxed).then(|| self.capture_request(&request));
        let started = Instant::now();
        let mut result = client.execute(request).await;
        let Some(log) = log else { return result; };

        let mut network_log = NetworkRequestLog {
            id: log.id,
            timestamp: log.timestamp,
            source: log.source,
            method: log.method,
            url: log.url,
            status: None,
            ok: None,
            duration_ms: Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            request_headers: log.request_headers,
            response_headers: vec![],
            request_body: log.request_body,
            request_body_truncated: log.request_body_truncated,
            response_body: None,
            response_body_truncated: false,
            error: None,
        };

        match &mut result {
            Ok(response) => {
                response.extensions_mut().insert(RequestLogId(network_log.id.clone()));
                network_log.status = Some(response.status().as_u16());
                network_log.ok = Some(response.status().is_success());
                network_log.response_headers = headers_to_debug(response.headers());
            }
            Err(error) => {
                network_log.status = error.status().map(|status| status.as_u16());
                network_log.ok = Some(false);
                network_log.error = Some(error_category(error).to_owned());
            }
        }

        self.push_log(network_log).await;

        result
    }

    async fn push_log(&self, log: NetworkRequestLog) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        let mut logs = self.logs.write().await;
        if !self.enabled.load(Ordering::Relaxed) { return; }
        if logs.len() >= MAX_NETWORK_LOGS {
            logs.pop_front();
        }
        logs.push_back(log);
    }

    fn capture_request(&self, request: &Request) -> PendingNetworkLog {
        let (request_body, request_body_truncated) = body_preview(request);

        PendingNetworkLog {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now().to_rfc3339(),
            source: request.url().host_str().unwrap_or("unknown").to_owned(),
            method: request.method().as_str().to_owned(),
            // Origins identify the service without logging credential-bearing URLs.
            url: request.url().origin().ascii_serialization(),
            request_headers: headers_to_debug(request.headers()),
            request_body,
            request_body_truncated,
        }
    }
}

#[derive(Debug)]
struct PendingNetworkLog {
    id: String,
    timestamp: String,
    source: String,
    method: String,
    url: String,
    request_headers: Vec<DebugHttpHeader>,
    request_body: Option<String>,
    request_body_truncated: bool,
}

fn omitted_body(bytes: usize) -> String {
    format!("<body omitted: {bytes} bytes>")
}

fn body_preview(request: &Request) -> (Option<String>, bool) {
    let Some(body) = request.body() else { return (None, false); };
    let Some(bytes) = body.as_bytes() else {
        return (Some("<streaming body omitted>".to_owned()), true);
    };
    if bytes.is_empty() { return (None, false); }
    (Some(omitted_body(bytes.len())), true)
}

fn error_category(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() { "request timed out" }
    else if error.is_connect() { "connection failed" }
    else if error.is_redirect() { "redirect failed" }
    else if error.is_status() { "HTTP status error" }
    else if error.is_decode() { "response decoding failed" }
    else if error.is_body() { "body transfer failed" }
    else { "request failed" }
}

fn headers_to_debug(headers: &HeaderMap) -> Vec<DebugHttpHeader> {
    headers
        .iter()
        .map(|(name, value)| {
            let raw_value = value.to_str().unwrap_or("<non-utf8>");
            DebugHttpHeader {
                name: name.as_str().to_owned(),
                value: sanitize_header_value(name.as_str(), raw_value),
            }
        })
        .collect()
}

fn sanitize_header_value(name: &str, value: &str) -> String {
    // Unknown/custom headers are confidential by default, not an incomplete denylist.
    match name.to_ascii_lowercase().as_str() {
        "content-type" => {
            let essence = value.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
            match essence.as_str() {
                "application/json" | "text/plain" | "text/html" | "text/event-stream" |
                "application/x-www-form-urlencoded" | "multipart/form-data" |
                "application/octet-stream" | "application/pdf" | "image/png" | "image/jpeg" => essence,
                _ => "<redacted>".to_owned(),
            }
        }
        "content-length" | "retry-after" => value.trim().parse::<u64>()
            .map(|number| number.to_string()).unwrap_or_else(|_| "<redacted>".to_owned()),
        "content-encoding" => {
            let encodings: Vec<_> = value.split(',').map(str::trim).collect();
            if encodings.iter().all(|encoding| matches!(*encoding, "gzip" | "deflate" | "br" | "zstd" | "identity")) {
                encodings.join(", ")
            } else { "<redacted>".to_owned() }
        },
        _ => "<redacted>".to_owned(),
    }
}
