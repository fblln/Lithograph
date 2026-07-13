//! Local embedded graph UI server (LIT-24.15): serves static explorer UI
//! assets and read-only graph APIs from the local machine only.
//!
//! The graph API is exposed at `POST /rpc` using the JSON-RPC 2.0
//! `tools/call` envelope the vendored graph explorer frontend expects
//! (`{jsonrpc, id, method: "tools/call", params: {name, arguments}}`,
//! response unwrapped from `result.content[].text`), translating to and
//! from the existing [`crate::mcp::WikiMcpServer`] request/response shapes
//! so no other graph code needs to change. Every other path falls back to
//! serving static files from a configured assets directory.
//!
//! Security posture: binds `127.0.0.1` only (never configurable to a
//! wider address -- there is no `--host` flag), rejects any request whose
//! `Host` or `Origin` header does not name this server's own loopback
//! authority (defeats DNS-rebinding-style attacks, not just relying on
//! browser same-origin policy), sends a strict `Content-Security-Policy`
//! with no external or inline script/style sources, and bounds every
//! request to a fixed time budget so a stuck handler cannot hang the
//! server indefinitely.

use crate::mcp::{McpRequest, WikiMcpServer};
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{BoxError, Json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;
use tower::ServiceBuilder;
use tower::timeout::TimeoutLayer;
use tower_http::services::ServeDir;

/// Every request is bounded to this time budget; a handler that exceeds it
/// is cancelled and the client receives `504 Gateway Timeout` instead of an
/// indefinitely hanging connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
     connect-src 'self'; img-src 'self' data:; font-src 'self'; object-src 'none'; \
     base-uri 'none'; frame-ancestors 'none'";

/// Binds the server's listening socket without accepting connections yet,
/// returning the bound address (the real port when `port` is `0`) and the
/// configured router. Split from [`run`] so tests and callers that need
/// the bound address up front don't have to go through the
/// graceful-shutdown-on-ctrl-c blocking loop `run` uses for the CLI.
pub async fn bind(
    repo_root: &Path,
    assets_dir: &Path,
    port: u16,
) -> std::io::Result<(tokio::net::TcpListener, SocketAddr, Router)> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let addr = listener.local_addr()?;
    Ok((listener, addr, router(repo_root, assets_dir)))
}

/// Binds and serves until the process receives `Ctrl-C`, writing the bound
/// address to `writer` first so the caller knows where to browse.
pub async fn run(
    repo_root: &Path,
    assets_dir: &Path,
    port: u16,
    writer: &mut impl Write,
) -> std::io::Result<()> {
    let (listener, addr, app) = bind(repo_root, assets_dir, port).await?;
    writeln!(writer, "Lithograph graph explorer serving on http://{addr}")?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

/// Builds the server's router: `POST /rpc` for the graph API, everything
/// else falls back to static files under `assets_dir`. `assets_dir` need
/// not exist -- missing files simply 404 rather than the server refusing
/// to start, so the graph API stays usable before a UI bundle is built.
fn router(repo_root: &Path, assets_dir: &Path) -> Router {
    let server = WikiMcpServer::new(repo_root);
    Router::new()
        .route("/rpc", post(rpc_handler))
        .fallback_service(ServeDir::new(assets_dir))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn(local_origin_guard))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_layer_error))
                .layer(TimeoutLayer::new(REQUEST_TIMEOUT)),
        )
        .with_state(server)
}

async fn handle_layer_error(error: BoxError) -> (StatusCode, String) {
    if error.is::<tower::timeout::error::Elapsed>() {
        (
            StatusCode::GATEWAY_TIMEOUT,
            "request exceeded the server's time budget and was cancelled".to_owned(),
        )
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unhandled server error: {error}"),
        )
    }
}

/// Rejects any request whose `Host` or `Origin` header does not name this
/// server's own loopback authority. `Host` is checked because DNS
/// rebinding attacks specifically target it (a public hostname resolved to
/// `127.0.0.1` after the browser's initial same-origin check passes);
/// `Origin` is checked for ordinary cross-origin `fetch`/XHR requests,
/// which browsers cannot be made to omit or forge.
async fn local_origin_guard(request: Request, next: Next) -> Response {
    if let Some(host) = request.headers().get(header::HOST)
        && !is_local_authority(host)
    {
        return (
            StatusCode::FORBIDDEN,
            "Host header must address this local server",
        )
            .into_response();
    }
    if let Some(origin) = request.headers().get(header::ORIGIN)
        && !is_local_origin(origin)
    {
        return (
            StatusCode::FORBIDDEN,
            "cross-origin requests are not permitted",
        )
            .into_response();
    }
    next.run(request).await
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    response
}

/// True when a `Host` header value names this server's own loopback
/// authority (`127.0.0.1[:port]` or `localhost[:port]`).
fn is_local_authority(value: &HeaderValue) -> bool {
    value.to_str().is_ok_and(matches_loopback_host)
}

/// True when an `Origin` header value is `http://` on a loopback host.
/// Parses out the host rather than string-prefix-matching, since
/// `http://127.0.0.1.evil.example` would otherwise pass a naive
/// `starts_with("http://127.0.0.1")` check.
fn is_local_origin(value: &HeaderValue) -> bool {
    value.to_str().is_ok_and(|text| {
        text.strip_prefix("http://")
            .is_some_and(matches_loopback_host)
    })
}

fn matches_loopback_host(authority: &str) -> bool {
    let authority = authority.split('/').next().unwrap_or(authority);
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _port)| host);
    host == "127.0.0.1" || host == "localhost"
}

/// A JSON-RPC 2.0 `tools/call` request, matching the vendored graph
/// explorer frontend's envelope.
#[derive(Debug, Clone, Deserialize)]
struct JsonRpcCall {
    id: Value,
    method: String,
    #[serde(default)]
    params: JsonRpcCallParams,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct JsonRpcCallParams {
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcOk {
    jsonrpc: &'static str,
    id: Value,
    result: JsonRpcToolResult,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcToolResult {
    content: Vec<JsonRpcContentBlock>,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcContentBlock {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcErr {
    jsonrpc: &'static str,
    id: Value,
    error: JsonRpcErrorBody,
}

#[derive(Debug, Clone, Serialize)]
struct JsonRpcErrorBody {
    code: i32,
    message: String,
}

fn json_rpc_error(id: Value, code: i32, message: String) -> Response {
    Json(JsonRpcErr {
        jsonrpc: "2.0",
        id,
        error: JsonRpcErrorBody { code, message },
    })
    .into_response()
}

/// Translates one JSON-RPC `tools/call` request into a
/// [`crate::mcp::WikiMcpServer`] tool call and back. The actual tool
/// handler is synchronous, in-memory graph computation (already
/// node/edge-budgeted by the tools themselves, e.g. `get_graph_layout` --
/// see LIT-24.16), so it runs on a blocking-pool thread rather than the
/// async reactor thread, and is bounded by this router's request-timeout
/// layer rather than a bespoke per-tool cancellation mechanism.
async fn rpc_handler(
    State(server): State<WikiMcpServer>,
    body: Result<Json<JsonRpcCall>, JsonRejection>,
) -> Response {
    let Json(call) = match body {
        Ok(json) => json,
        Err(rejection) => return json_rpc_error(Value::Null, -32700, rejection.to_string()),
    };
    if call.method != "tools/call" {
        return json_rpc_error(
            call.id,
            -32601,
            format!("unknown method `{}`; expected `tools/call`", call.method),
        );
    }
    let request = McpRequest {
        id: call.id,
        tool: call.params.name,
        params: call.params.arguments,
    };
    let response = match tokio::task::spawn_blocking(move || server.handle(request)).await {
        Ok(response) => response,
        Err(error) => {
            return json_rpc_error(
                Value::Null,
                -32603,
                format!("tool handler task failed: {error}"),
            );
        }
    };
    if let Some(message) = response.error {
        return json_rpc_error(response.id, -32000, message);
    }
    let text = match serde_json::to_string(&response.result.unwrap_or(Value::Null)) {
        Ok(text) => text,
        Err(error) => {
            return json_rpc_error(
                response.id,
                -32603,
                format!("failed to serialize tool result: {error}"),
            );
        }
    };
    Json(JsonRpcOk {
        jsonrpc: "2.0",
        id: response.id,
        result: JsonRpcToolResult {
            content: vec![JsonRpcContentBlock { kind: "text", text }],
        },
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request as HttpRequest;
    use serde_json::json;
    use tower::ServiceExt;

    fn runtime() -> tokio::runtime::Runtime {
        #[allow(clippy::expect_used)]
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("building a current-thread test runtime never fails in practice")
    }

    fn fixture_repo() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        crate::orchestrate::run_init(temp.path(), &crate::generation::MockModel, "mock", "v1")?;
        Ok(temp)
    }

    fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                std::fs::create_dir_all(&target)?;
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }

    #[allow(clippy::unwrap_used)]
    async fn rpc_request(app: Router, body: Value) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                HttpRequest::post("/rpc")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[test]
    fn rpc_endpoint_round_trips_a_tool_call() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("no-assets-here"));
        let (status, body) = runtime().block_on(rpc_request(
            app,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "get_graph_schema", "arguments": {} }
            }),
        ));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.get("jsonrpc"), Some(&json!("2.0")));
        assert_eq!(body.get("id"), Some(&json!(1)));
        let text = body
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(|content| content.get(0))
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
            .ok_or("expected result.content[0].text")?;
        let schema: Value = serde_json::from_str(text)?;
        assert!(schema.get("node_labels").is_some());
        Ok(())
    }

    #[test]
    fn rpc_endpoint_reports_tool_errors_as_json_rpc_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("no-assets-here"));
        let (status, body) = runtime().block_on(rpc_request(
            app,
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "get_graph_layout", "arguments": { "center_node": "does-not-exist" } }
            }),
        ));
        assert_eq!(status, StatusCode::OK);
        assert!(body.get("error").is_some());
        assert!(body.get("result").is_none());
        Ok(())
    }

    #[test]
    fn rpc_endpoint_rejects_unknown_methods() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("no-assets-here"));
        let (status, body) = runtime().block_on(rpc_request(
            app,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }),
        ));
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body.get("error").and_then(|error| error.get("code")),
            Some(&json!(-32601))
        );
        Ok(())
    }

    #[test]
    fn missing_assets_directory_404s_instead_of_failing_to_start()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("nonexistent-assets"));
        #[allow(clippy::unwrap_used)]
        let response = runtime()
            .block_on(app.oneshot(HttpRequest::get("/index.html").body(Body::empty()).unwrap()))
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        Ok(())
    }

    #[test]
    fn responses_carry_the_strict_csp_and_hardening_headers()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("no-assets-here"));
        #[allow(clippy::unwrap_used)]
        let response = runtime()
            .block_on(app.oneshot(HttpRequest::get("/index.html").body(Body::empty()).unwrap()))
            .unwrap();
        assert!(
            response
                .headers()
                .get(header::CONTENT_SECURITY_POLICY)
                .is_some()
        );
        assert!(
            response
                .headers()
                .get(HeaderName::from_static("x-frame-options"))
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn cross_origin_requests_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("no-assets-here"));
        #[allow(clippy::unwrap_used)]
        let response = runtime()
            .block_on(
                app.oneshot(
                    HttpRequest::post("/rpc")
                        .header(header::ORIGIN, "http://evil.example")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        Ok(())
    }

    #[test]
    fn dns_rebinding_style_host_headers_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let app = router(repo.path(), &repo.path().join("no-assets-here"));
        #[allow(clippy::unwrap_used)]
        let response = runtime()
            .block_on(
                app.oneshot(
                    HttpRequest::get("/index.html")
                        .header(header::HOST, "attacker-controlled.example")
                        .body(Body::empty())
                        .unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        Ok(())
    }

    #[test]
    fn matches_loopback_host_rejects_prefix_confusable_hosts() {
        assert!(matches_loopback_host("127.0.0.1"));
        assert!(matches_loopback_host("127.0.0.1:4317"));
        assert!(matches_loopback_host("localhost"));
        assert!(matches_loopback_host("localhost:4317"));
        assert!(!matches_loopback_host("127.0.0.1.evil.example"));
        assert!(!matches_loopback_host("evil.example"));
        assert!(!matches_loopback_host("notlocalhost:4317"));
    }

    #[test]
    fn is_local_origin_rejects_non_http_and_confusable_schemes() {
        assert!(is_local_origin(&HeaderValue::from_static(
            "http://127.0.0.1:4317"
        )));
        assert!(!is_local_origin(&HeaderValue::from_static(
            "https://127.0.0.1:4317"
        )));
        assert!(!is_local_origin(&HeaderValue::from_static(
            "http://127.0.0.1.evil.example"
        )));
        assert!(!is_local_origin(&HeaderValue::from_static("null")));
    }

    #[test]
    fn bind_reports_the_real_ephemeral_port_and_serves_a_real_request()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        runtime().block_on(async {
            let (listener, addr, app) = bind(repo.path(), &repo.path().join("no-assets-here"), 0)
                .await
                .map_err(|error| -> Box<dyn std::error::Error> { Box::new(error) })?;
            assert_ne!(addr.port(), 0);
            let server = tokio::spawn(axum::serve(listener, app).into_future());

            let response = tokio::task::spawn_blocking(move || {
                ureq::post(&format!("http://{addr}/rpc")).send_json(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "tools/call",
                    "params": { "name": "get_graph_schema", "arguments": {} }
                }))
            })
            .await?;
            server.abort();
            let response = response?;
            assert_eq!(response.status(), 200);
            Ok(())
        })
    }
}
