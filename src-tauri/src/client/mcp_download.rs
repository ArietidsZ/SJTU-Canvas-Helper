//! Credential-free, bounded downloads for MCP file explanations only.
//!
//! Callers must obtain metadata from the authenticated Canvas API, not MCP
//! arguments. Exact-host validation is not DNS/IP pinning or a parser sandbox.
//! This module intentionally depends only on reqwest, tokio, and std so its
//! policy tests can also run in a small offline harness.
use reqwest::{header, Client, StatusCode, Url};
use std::{fmt, time::Duration};

pub(super) const MAX_BYTES: usize = 20 * 1024 * 1024;
pub(super) const MAX_TEXT_CHARS: usize = 128_000;
const MAX_NAME_BYTES: usize = 1024;
const MAX_URL_BYTES: usize = 16 * 1024;
const MAX_REDIRECTS: usize = 5;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DownloadError {
    Metadata,
    Url,
    Redirect,
    TooLarge,
    SizeMismatch,
    Network,
    Timeout,
    HttpStatus,
    ContentEncoding,
    TextTooLarge,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never include URL/query strings, response bodies, or underlying errors.
        f.write_str(match self {
            Self::Metadata => "MCP file metadata is unavailable or not allowed",
            Self::Url => "MCP file download URL is not allowed",
            Self::Redirect => "MCP file redirect is not allowed",
            Self::TooLarge => "MCP file exceeds the 20 MiB download limit",
            Self::SizeMismatch => "MCP file size does not match server metadata",
            Self::Network => "MCP file download failed",
            Self::Timeout => "MCP file download timed out",
            Self::HttpStatus => "MCP file download did not return a complete file",
            Self::ContentEncoding => "MCP file download encoding is not supported",
            Self::TextTooLarge => "MCP file text exceeds the 128000 character limit",
        })
    }
}

impl std::error::Error for DownloadError {}
type Result<T> = std::result::Result<T, DownloadError>;

pub(super) fn validate_metadata(
    locked: bool,
    locked_for_user: Option<bool>,
    size: u64,
    external: bool,
    display_name: &str,
) -> Result<String> {
    // An omitted per-user lock flag is unknown, not evidence of permission.
    if locked || locked_for_user != Some(false) || external {
        return Err(DownloadError::Metadata);
    }
    if size > MAX_BYTES as u64 {
        return Err(DownloadError::TooLarge);
    }
    if display_name.is_empty() || display_name.len() > MAX_NAME_BYTES {
        return Err(DownloadError::Metadata);
    }
    let extension = std::path::Path::new(display_name)
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "pdf" | "docx") {
        return Err(DownloadError::Metadata);
    }
    Ok(extension)
}

fn validate_url(url: &Url) -> Result<()> {
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !matches!(url.host_str(), Some("oc.sjtu.edu.cn" | "jicanvas.com"))
    {
        return Err(DownloadError::Url);
    }
    Ok(())
}

fn has_userinfo(raw: &str) -> bool {
    // Network-path references must be recognized before any port colon.
    let authority = raw.strip_prefix("//").or_else(|| {
        raw.split_once("://").and_then(|(scheme, rest)| {
            (!scheme.contains(['/', '?', '#'])).then_some(rest)
        })
    });
    authority.is_some_and(|rest| {
        rest.trim_start_matches('/').split(['/', '?', '#'])
            .next().is_some_and(|part| part.contains('@'))
    })
}

fn initial_url(raw: &str) -> Result<Url> {
    // Reject URL-parser normalizations of ambiguous control/backslash input.
    if !raw.starts_with("https://")
        || has_userinfo(raw)
        || raw.len() > MAX_URL_BYTES
        || raw.chars().any(|character| character.is_control() || character.is_whitespace())
        || raw.contains('\\')
    {
        return Err(DownloadError::Url);
    }
    let url = Url::parse(raw).map_err(|_| DownloadError::Url)?;
    validate_url(&url)?;
    Ok(url)
}

fn redirect_target(current: &Url, location: &str, followed: usize) -> Result<Url> {
    // Do not allow URL joining to normalize shorthand scheme forms such as
    // "https:@host" before the raw userinfo check can see their authority.
    let abbreviated_scheme = location.split(['/', '?', '#']).next()
        .and_then(|first| first.find(':'))
        .is_some_and(|colon| !location[colon..].starts_with("://"));
    if abbreviated_scheme
        || has_userinfo(location)
        || followed >= MAX_REDIRECTS
        || location.len() > MAX_URL_BYTES
        || location.chars().any(|character| character.is_control() || character.is_whitespace())
        || location.contains('\\')
    {
        return Err(DownloadError::Redirect);
    }
    let next = current.join(location).map_err(|_| DownloadError::Redirect)?;
    // Reuse initial validation after joining, including the resulting URL limit.
    initial_url(next.as_str()).map_err(|_| DownloadError::Redirect)
}

fn download_client() -> Result<Client> {
    Client::builder()
        .no_proxy()
        .referer(false)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(DOWNLOAD_TIMEOUT)
        // A fresh client has no cookie store, app headers, or bearer token.
        .build()
        .map_err(|_| DownloadError::Network)
}

fn request(client: &Client, target: Url) -> Result<reqwest::Request> {
    client
        .get(target)
        .header(header::ACCEPT_ENCODING, "identity")
        .build()
        .map_err(|_| DownloadError::Network)
}

struct BodyBuffer {
    bytes: Vec<u8>,
    expected: usize,
}

impl BodyBuffer {
    fn new(expected: u64, content_length: Option<u64>) -> Result<Self> {
        if expected > MAX_BYTES as u64 || content_length.is_some_and(|size| size > MAX_BYTES as u64) {
            return Err(DownloadError::TooLarge);
        }
        if content_length.is_some_and(|size| size != expected) {
            return Err(DownloadError::SizeMismatch);
        }
        Ok(Self {
            bytes: Vec::new(),
            expected: expected as usize,
        })
    }

    fn push(&mut self, chunk: &[u8]) -> Result<()> {
        if chunk.len() > MAX_BYTES.saturating_sub(self.bytes.len()) {
            return Err(DownloadError::TooLarge);
        }
        if chunk.len() > self.expected.saturating_sub(self.bytes.len()) {
            return Err(DownloadError::SizeMismatch);
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    fn finish(self) -> Result<Vec<u8>> {
        if self.bytes.len() != self.expected {
            return Err(DownloadError::SizeMismatch);
        }
        Ok(self.bytes)
    }
}

pub(super) fn validate_text(text: &str) -> Result<()> {
    if text.chars().take(MAX_TEXT_CHARS + 1).count() > MAX_TEXT_CHARS {
        return Err(DownloadError::TextTooLarge);
    }
    Ok(())
}

pub(super) async fn download(raw_url: &str, expected_size: u64) -> Result<Vec<u8>> {
    let target = initial_url(raw_url)?;
    // Validate metadata size even if an internal caller forgot its own check.
    BodyBuffer::new(expected_size, None)?;
    let client = download_client()?;
    // A single budget covers all redirect hops and streamed body reads, rather
    // than renewing the full timeout for every request in the chain.
    tokio::time::timeout(DOWNLOAD_TIMEOUT, download_inner(&client, target, expected_size))
        .await
        .map_err(|_| DownloadError::Timeout)?
}

async fn download_inner(client: &Client, mut target: Url, expected_size: u64) -> Result<Vec<u8>> {
    let mut followed = 0;
    loop {
        // Automatic redirects are disabled. Every target is checked before I/O.
        validate_url(&target)?;
        let mut response = client
            .execute(request(client, target.clone())?)
            .await
            .map_err(|_| DownloadError::Network)?;
        if response.status().is_redirection() {
            if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Err(DownloadError::Redirect);
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or(DownloadError::Redirect)?;
            target = redirect_target(&target, location, followed)?;
            followed += 1;
            // Redirect response bodies and Set-Cookie values are never consumed.
            continue;
        }
        if response.status() != StatusCode::OK {
            return Err(DownloadError::HttpStatus);
        }
        for encoding in response.headers().get_all(header::CONTENT_ENCODING).iter() {
            if !encoding.to_str().is_ok_and(|value| value.eq_ignore_ascii_case("identity")) {
                return Err(DownloadError::ContentEncoding);
            }
        }
        let mut body = BodyBuffer::new(expected_size, response.content_length())?;
        while let Some(chunk) = response.chunk().await.map_err(|_| DownloadError::Network)? {
            body.push(&chunk)?;
        }
        return body.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_https_canvas_origins_only() {
        for raw in [
            "https://oc.sjtu.edu.cn/files/1/download?verifier=signed",
            "https://jicanvas.com:443/files/2/download",
        ] {
            assert!(initial_url(raw).is_ok(), "{raw}");
        }
        for raw in [
            "http://oc.sjtu.edu.cn/files/1", "https://localhost/",
            "https://127.0.0.1/", "https://[::1]/", "https://169.254.169.254/",
            "https://oc.sjtu.edu.cn.evil.test/", "https://sub.oc.sjtu.edu.cn/",
            "https://oc.sjtu.edu.cn./", "https://evil.test/?host=oc.sjtu.edu.cn",
            "https://user:secret@oc.sjtu.edu.cn/", "https://user@oc.sjtu.edu.cn/",
            "https://@oc.sjtu.edu.cn/", "https://:@oc.sjtu.edu.cn/", "https:oc.sjtu.edu.cn/",
            "https://oc.sjtu.edu.cn:8443/", "https://oc.sjtu.edu.cn/#fragment",
            "file:///etc/passwd", "/local/file.pdf", "data:text/plain,test",
            " https://oc.sjtu.edu.cn/", "https://oc.sjtu.edu.cn/\npath",
            "https://oc.sjtu.edu.cn\\@evil.test/",
        ] {
            assert!(initial_url(raw).is_err(), "{raw}");
        }
        assert!(initial_url(&format!("https://oc.sjtu.edu.cn/?{}", "x".repeat(MAX_URL_BYTES))).is_err());
    }

    #[test]
    fn every_redirect_target_and_limit_are_checked_before_request() {
        let current = initial_url("https://oc.sjtu.edu.cn/files/1/download").unwrap();
        assert!(redirect_target(&current, "/files/1/raw?verifier=signed", 0).is_ok());
        assert!(redirect_target(&current, "https://jicanvas.com/files/1", MAX_REDIRECTS - 1).is_ok());
        assert_eq!(redirect_target(&current, "/same-host", MAX_REDIRECTS), Err(DownloadError::Redirect));
        for location in [
            "https://evil.test/", "//127.0.0.1/", "http://oc.sjtu.edu.cn/",
            "https://user:secret@jicanvas.com/", "https://jicanvas.com:8443/",
            "https://@jicanvas.com/", "//@jicanvas.com/", "https:@jicanvas.com/",
            "//@oc.sjtu.edu.cn:443/path", "//:@oc.sjtu.edu.cn:443/path",
            "//user@oc.sjtu.edu.cn:443/path", "https:////@jicanvas.com/",
            "file:///tmp/test", "//jicanvas.com.evil.test/", "/safe\nunsafe",
        ] {
            assert_eq!(redirect_target(&current, location, 0), Err(DownloadError::Redirect));
        }
    }

    #[test]
    fn missing_locked_or_synthetic_metadata_fails_closed() {
        assert_eq!(validate_metadata(false, Some(false), 1, false, "file.PDF").unwrap(), "pdf");
        assert_eq!(validate_metadata(false, Some(false), 1, false, "file.docx").unwrap(), "docx");
        for (locked, per_user, external) in [(true, Some(false), false), (false, None, false),
            (false, Some(true), false), (false, Some(false), true)] {
            assert_eq!(validate_metadata(locked, per_user, 1, external, "file.pdf"), Err(DownloadError::Metadata));
        }
        assert_eq!(validate_metadata(false, Some(false), MAX_BYTES as u64 + 1, false, "file.pdf"), Err(DownloadError::TooLarge));
        for name in ["file.exe", "file", "", "file.pdf.exe"] {
            assert!(validate_metadata(false, Some(false), 1, false, name).is_err());
        }
    }

    #[test]
    fn content_length_never_replaces_counting_streamed_bytes() {
        assert!(matches!(BodyBuffer::new(1, Some(MAX_BYTES as u64 + 1)), Err(DownloadError::TooLarge)));
        assert!(matches!(BodyBuffer::new(MAX_BYTES as u64 + 1, None), Err(DownloadError::TooLarge)));
        assert!(matches!(BodyBuffer::new(4, Some(3)), Err(DownloadError::SizeMismatch)));
        let mut body = BodyBuffer::new(4, None).unwrap();
        body.push(b"ab").unwrap();
        body.push(b"cd").unwrap();
        assert_eq!(body.push(b"e"), Err(DownloadError::SizeMismatch));
        assert_eq!(body.finish().unwrap(), b"abcd".to_vec());
        let mut short = BodyBuffer::new(4, None).unwrap();
        short.push(b"abc").unwrap();
        assert_eq!(short.finish(), Err(DownloadError::SizeMismatch));
    }

    #[test]
    fn byte_limit_is_inclusive_and_rejection_does_not_append() {
        let mut body = BodyBuffer::new(MAX_BYTES as u64, None).unwrap();
        body.push(&vec![0; MAX_BYTES]).unwrap();
        assert_eq!(body.push(&[1]), Err(DownloadError::TooLarge));
        assert_eq!(body.finish().unwrap().len(), MAX_BYTES);
        assert!(BodyBuffer::new(0, Some(0)).unwrap().finish().unwrap().is_empty());
    }

    #[test]
    fn text_limit_counts_unicode_characters_and_is_inclusive() {
        let mut text = "文".repeat(MAX_TEXT_CHARS);
        assert!(validate_text(&text).is_ok());
        text.push('字');
        assert_eq!(validate_text(&text), Err(DownloadError::TextTooLarge));
    }

    #[test]
    fn isolated_requests_have_no_application_credentials_or_referer() {
        let client = download_client().unwrap();
        let request = request(&client, initial_url("https://oc.sjtu.edu.cn/file?verifier=signed").unwrap()).unwrap();
        for name in [header::AUTHORIZATION, header::COOKIE, header::PROXY_AUTHORIZATION, header::REFERER] {
            assert!(!request.headers().contains_key(name));
        }
        assert_eq!(request.headers()[header::ACCEPT_ENCODING], "identity");
    }

    #[tokio::test]
    async fn rejected_inputs_never_reach_transport_or_model() {
        assert_eq!(download("https://evil.test/file?token=secret", 1).await, Err(DownloadError::Url));
        assert_eq!(download("https://oc.sjtu.edu.cn/file", MAX_BYTES as u64 + 1).await, Err(DownloadError::TooLarge));
    }

    #[test]
    fn public_errors_never_contain_input_or_transport_details() {
        for error in [DownloadError::Metadata, DownloadError::Url, DownloadError::Redirect,
            DownloadError::TooLarge, DownloadError::SizeMismatch, DownloadError::Network,
            DownloadError::Timeout, DownloadError::HttpStatus, DownloadError::ContentEncoding,
            DownloadError::TextTooLarge] {
            let message = error.to_string();
            assert!(!message.contains("https://"));
            assert!(!message.contains("token="));
            assert!(!message.contains("verifier="));
        }
    }
}
