//! The `fetch_url` tool: GET a URL, gate the host, hard-block SSRF, return readable text.
use crate::policy::{HostDecision, NetworkPolicy};
use crate::ssrf::SsrfGuard;
use agent_tools::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use url::Url;

const USER_AGENT: &str = "agent-http/0.1 (+local agent runtime)";
const MAX_REDIRECTS: usize = 5;
const MAX_DOWNLOAD: usize = 2 * 1024 * 1024; // 2 MiB

pub struct FetchUrl {
    policy: NetworkPolicy,
    guard: SsrfGuard,
}

impl FetchUrl {
    pub fn new(policy: NetworkPolicy) -> Self {
        Self { policy, guard: SsrfGuard::strict() }
    }

    #[cfg(test)]
    pub(crate) fn with_guard(policy: NetworkPolicy, guard: SsrfGuard) -> Self {
        Self { policy, guard }
    }
}

/// Parse the `url` arg, accepting only http/https. Used by both `intent` and `execute`.
fn parse_url(args: &Value) -> Result<Url, ToolError> {
    let s = args
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("missing 'url' string".into()))?;
    let url = Url::parse(s).map_err(|e| ToolError::InvalidArgs(format!("invalid url: {e}")))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(ToolError::InvalidArgs(format!(
            "unsupported scheme '{other}': only http/https are allowed"
        ))),
    }
}

/// A reqwest DNS resolver pinned to ONE already-validated address. We resolve + SSRF-check
/// ourselves, then install this so reqwest connects only to the IP we approved (no second
/// lookup → no DNS-rebinding window). The real URL port is baked in, so reqwest never falls
/// back to a scheme-default port (unlike `ClientBuilder::resolve`, which ignores the port).
struct FixedResolver {
    ip: IpAddr,
    port: u16,
}

impl Resolve for FixedResolver {
    fn resolve(&self, _name: Name) -> Resolving {
        let addr = SocketAddr::new(self.ip, self.port);
        let addrs: Addrs = Box::new(std::iter::once(addr));
        Box::pin(std::future::ready(Ok(addrs)))
    }
}

/// Resolve `host:port` to socket addresses, honoring cancellation.
async fn resolve(host: &str, port: u16, cancel: &CancellationToken) -> Result<Vec<SocketAddr>, ToolError> {
    let lookup = tokio::net::lookup_host((host, port));
    let addrs = tokio::select! {
        _ = cancel.cancelled() => return Err(ToolError::Timeout),
        r = lookup => r.map_err(|e| ToolError::NotFound(format!("dns for {host}: {e}")))?,
    };
    Ok(addrs.collect())
}

/// Read a response body, aborting past `MAX_DOWNLOAD`. Returns (bytes, hit_cap).
async fn read_capped(
    resp: reqwest::Response,
    cancel: &CancellationToken,
) -> Result<(Vec<u8>, bool), ToolError> {
    let mut buf = Vec::new();
    let mut stream = resp.bytes_stream();
    loop {
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err(ToolError::Timeout),
            n = stream.next() => n,
        };
        match next {
            Some(Ok(chunk)) => {
                buf.extend_from_slice(&chunk);
                if buf.len() > MAX_DOWNLOAD {
                    buf.truncate(MAX_DOWNLOAD);
                    return Ok((buf, true));
                }
            }
            Some(Err(e)) => return Err(ToolError::Failed { message: format!("body read: {e}"), stderr: None }),
            None => return Ok((buf, false)),
        }
    }
}

fn human(bytes: usize) -> String {
    if bytes >= 1024 { format!("{:.1} KB", bytes as f64 / 1024.0) } else { format!("{bytes} B") }
}

#[async_trait]
impl Tool for FetchUrl {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Fetch a web page or document over HTTP(S) (GET only) and return its readable \
         text/JSON. Use for docs and reference pages. Non-allowlisted hosts require approval."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "fetch_url".into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Absolute http(s) URL to GET." }
                },
                "required": ["url"]
            }),
        }
    }

    fn intent(&self, args: &Value) -> Result<ToolIntent, ToolError> {
        let url = parse_url(args)?;
        let host = url
            .host_str()
            .ok_or_else(|| ToolError::InvalidArgs("url has no host".into()))?;
        let access = match self.policy.decide(host) {
            HostDecision::Allow => Access::Read,
            HostDecision::Ask => Access::Write,
        };
        Ok(ToolIntent {
            tool: "fetch_url".into(),
            access,
            paths: vec![],
            command: None,
            summary: format!("GET {url}"),
        })
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        let mut url = parse_url(&args)?;
        let mut hops = 0usize;

        loop {
            let host = url
                .host_str()
                .ok_or_else(|| ToolError::InvalidArgs("url has no host".into()))?
                .to_string();
            let port = url
                .port_or_known_default()
                .ok_or_else(|| ToolError::InvalidArgs("url has no port".into()))?;

            // Resolve and validate EVERY address before connecting (SSRF hard floor).
            let addrs = resolve(&host, port, &ctx.cancel).await?;
            if addrs.is_empty() {
                return Err(ToolError::NotFound(format!("no addresses for {host}")));
            }
            for a in &addrs {
                self.guard.check(a.ip()).map_err(|ip| {
                    ToolError::Denied(format!("address {ip} for {host} is blocked (SSRF guard)"))
                })?;
            }

            // Pin reqwest to the validated address; disable auto-redirects so we re-validate each hop.
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .dns_resolver(Arc::new(FixedResolver { ip: addrs[0].ip(), port }))
                .timeout(ctx.timeout)
                .user_agent(USER_AGENT)
                .build()
                .map_err(|e| ToolError::Failed { message: format!("http client: {e}"), stderr: None })?;

            let send = client.get(url.clone()).send();
            let resp = tokio::select! {
                _ = ctx.cancel.cancelled() => return Err(ToolError::Timeout),
                r = send => r.map_err(|e| ToolError::Failed { message: format!("request: {e}"), stderr: None })?,
            };

            let status = resp.status();

            if status.is_redirection() {
                if let Some(loc) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                {
                    hops += 1;
                    if hops > MAX_REDIRECTS {
                        return Err(ToolError::Failed { message: "too many redirects".into(), stderr: None });
                    }
                    url = url
                        .join(loc)
                        .map_err(|e| ToolError::Failed { message: format!("bad redirect '{loc}': {e}"), stderr: None })?;
                    if !matches!(url.scheme(), "http" | "https") {
                        return Err(ToolError::Denied(format!("redirect to non-http scheme '{}'", url.scheme())));
                    }
                    continue; // re-resolve + re-validate the new target
                }
                // Redirect status without a Location header: fall through and render the body.
            }

            let ctype = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let (body, hit_cap) = read_capped(resp, &ctx.cancel).await?;
            let rendered = crate::content::render(&ctype, &body, &url, hit_cap)?;

            let final_url = url.to_string();
            let content = format!("GET {final_url} -> {}\n\n{}", status.as_u16(), rendered.text);
            let display = Display::Text(format!(
                "GET {final_url} -> {} ({} {})",
                status.as_u16(),
                human(body.len()),
                rendered.kind
            ));
            return Ok(ToolOutput { content, display: Some(display) });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_policy::{Decision, PolicyEngine, RulePolicy};
    use agent_tools::{Access, Tool};
    use serde_json::json;
    use std::path::PathBuf;

    fn rule_policy() -> RulePolicy {
        RulePolicy {
            workspace: PathBuf::from("/work"),
            command_allowlist: vec![],
            command_denylist: vec![],
        }
    }

    #[test]
    fn schema_and_name_are_stable() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        assert_eq!(t.name(), "fetch_url");
        assert_eq!(t.schema().parameters["properties"]["url"]["type"], "string");
    }

    #[test]
    fn allowlisted_host_maps_to_read_and_rule_policy_allows() {
        let t = FetchUrl::new(NetworkPolicy::new(&["example.com".to_string()]));
        let intent = t.intent(&json!({"url": "https://example.com/page"})).unwrap();
        assert!(matches!(intent.access, Access::Read));
        assert!(intent.paths.is_empty());
        assert!(matches!(rule_policy().check(&intent), Decision::Allow));
    }

    #[test]
    fn unknown_host_maps_to_write_and_rule_policy_asks() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        let intent = t.intent(&json!({"url": "https://example.com/"})).unwrap();
        assert!(matches!(intent.access, Access::Write));
        assert!(matches!(rule_policy().check(&intent), Decision::Ask));
    }

    #[test]
    fn non_http_scheme_is_invalid_args() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        let err = t.intent(&json!({"url": "file:///etc/passwd"})).unwrap_err();
        assert!(matches!(err, agent_tools::ToolError::InvalidArgs(_)));
    }

    #[test]
    fn missing_url_is_invalid_args() {
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        assert!(matches!(t.intent(&json!({})).unwrap_err(), agent_tools::ToolError::InvalidArgs(_)));
    }

    use crate::ssrf::SsrfGuard;
    use agent_tools::ToolCtx;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ctx() -> ToolCtx {
        ToolCtx {
            workspace: std::path::PathBuf::from("/tmp"),
            timeout: Duration::from_secs(10),
            cancel: CancellationToken::new(),
        }
    }

    // Allow loopback so we can talk to the wiremock server; production always uses strict().
    fn permissive() -> FetchUrl {
        FetchUrl::with_guard(NetworkPolicy::new(&[]), SsrfGuard::allow_all())
    }

    #[tokio::test]
    async fn fetches_html_and_returns_readable_text() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/doc"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string("<html><body><h1>Hi</h1><p>Readable body here.</p></body></html>"))
            .mount(&server).await;

        let url = format!("{}/doc", server.uri());
        let out = permissive().execute(json!({ "url": url }), &ctx()).await.unwrap();
        assert!(out.content.contains("Readable body here"));
        assert!(out.content.starts_with("GET "));
    }

    #[tokio::test]
    async fn follows_redirect_then_fetches() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/from"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/to"))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/to"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "text/plain").set_body_string("landed"))
            .mount(&server).await;

        let url = format!("{}/from", server.uri());
        let out = permissive().execute(json!({ "url": url }), &ctx()).await.unwrap();
        assert!(out.content.contains("landed"));
        assert!(out.content.contains("/to"), "final_url should reflect the redirect target");
    }

    #[tokio::test]
    async fn binary_content_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/blob"))
            .respond_with(ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(vec![0u8, 1, 2, 3]))
            .mount(&server).await;

        let url = format!("{}/blob", server.uri());
        let err = permissive().execute(json!({ "url": url }), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::Failed { .. }));
    }

    #[tokio::test]
    async fn strict_guard_blocks_loopback_target() {
        // Strict guard (production default) must refuse a loopback URL with Denied.
        let t = FetchUrl::new(NetworkPolicy::new(&[]));
        let err = t.execute(json!({ "url": "http://127.0.0.1:9/" }), &ctx()).await.unwrap_err();
        assert!(matches!(err, ToolError::Denied(_)), "expected SSRF Denied, got {err:?}");
    }
}
