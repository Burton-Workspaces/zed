use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Context as _;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as PathParams, Query, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE, HOST};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::json;
use tokio_util::io::ReaderStream;
use url::Url;

const FORWARDED_REQUEST_SKIP: &[&str] = &[
    "host",
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "cookie",
    "accept-encoding",
    "content-length",
];

const FORWARDED_RESPONSE_ALLOW: &[&str] = &["content-type", "content-length", "cache-control"];

#[derive(Clone)]
pub struct AppState {
    search_dirs: Vec<PathBuf>,
    version: String,
    public_url: Option<String>,
    extensions_upstream: Url,
    http_client: reqwest::Client,
}

impl AppState {
    pub fn new(
        search_dirs: Vec<PathBuf>,
        version: String,
        public_url: Option<String>,
        extensions_upstream: Url,
    ) -> anyhow::Result<Self> {
        let http_client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .context("building HTTP client")?;
        Ok(Self {
            search_dirs,
            version,
            public_url: public_url.map(|value| value.trim_end_matches('/').to_string()),
            extensions_upstream,
            http_client,
        })
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/releases/{channel}/{version}/asset", get(release_asset))
        .route("/files/{name}", get(serve_file))
        .route("/extensions", get(proxy_extensions))
        .route("/extensions/{*rest}", get(proxy_extensions))
        .layer(axum::middleware::from_fn(log_request))
        .with_state(state)
}

pub async fn serve(state: AppState, listen: SocketAddr) -> anyhow::Result<()> {
    let search_dirs = state
        .search_dirs
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("binding {listen}"))?;
    eprintln!(
        "Burton update server on http://{listen}\n  version: {}\n  files:   {search_dirs}\n  extensions upstream: {}\nPoint a build at this host with ZED_SERVER_URL=http://{listen}",
        state.version, state.extensions_upstream
    );
    axum::serve(listener, router(Arc::new(state)))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("\nstopped");
        })
        .await
        .context("server error")
}

/// Walk from this crate to the Zed/Burton repo root.
pub fn repo_root() -> PathBuf {
    let compiled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    if compiled.join("burton/branding.toml").is_file() {
        return compiled.canonicalize().unwrap_or(compiled);
    }
    let Ok(mut current) = std::env::current_dir() else {
        return compiled;
    };
    loop {
        if current.join("burton/branding.toml").is_file() {
            return current;
        }
        if !current.pop() {
            return compiled;
        }
    }
}

pub fn crate_version(root: &Path) -> anyhow::Result<String> {
    let path = root.join("crates/zed/Cargo.toml");
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(version) = rest
            .strip_prefix('"')
            .and_then(|value| value.split('"').next())
        {
            return Ok(version.to_string());
        }
    }
    anyhow::bail!("could not read version from {}", path.display())
}

pub fn public_origin(
    configured: Option<&str>,
    forwarded_proto: Option<&str>,
    forwarded_host: Option<&str>,
    host: Option<&str>,
) -> String {
    if let Some(url) = configured.filter(|value| !value.is_empty()) {
        return url.trim_end_matches('/').to_string();
    }
    let host = first_forwarded_value(forwarded_host)
        .or_else(|| host.filter(|value| !value.is_empty()))
        .unwrap_or("127.0.0.1");
    let proto = first_forwarded_value(forwarded_proto).unwrap_or("http");
    format!("{proto}://{host}")
}

fn first_forwarded_value(header: Option<&str>) -> Option<&str> {
    header.and_then(|value| {
        value
            .split(',')
            .map(str::trim)
            .find(|part| !part.is_empty())
    })
}

pub fn asset_file_names(asset: &str, os: &str, arch: &str) -> Option<&'static [&'static str]> {
    match (asset, os, arch) {
        ("zed", "linux", "x86_64") => Some(&["burton-linux-x86_64.tar.gz"]),
        ("zed", "linux", "aarch64") => Some(&["burton-linux-aarch64.tar.gz"]),
        ("zed", "macos", "x86_64") => Some(&["Burton-x86_64.dmg"]),
        ("zed", "macos", "aarch64") => Some(&["Burton-aarch64.dmg"]),
        ("zed", "windows", "x86_64") => Some(&["Burton-x86_64.exe"]),
        ("zed", "windows", "aarch64") => Some(&["Burton-aarch64.exe"]),
        ("zed-remote-server", "linux", "x86_64") => Some(&["zed-remote-server-linux-x86_64.gz"]),
        ("zed-remote-server", "linux", "aarch64") => Some(&["zed-remote-server-linux-aarch64.gz"]),
        ("zed-remote-server", "macos", "x86_64") => Some(&["zed-remote-server-macos-x86_64.gz"]),
        ("zed-remote-server", "macos", "aarch64") => Some(&["zed-remote-server-macos-aarch64.gz"]),
        ("zed-remote-server", "windows", "x86_64") => {
            Some(&["zed-remote-server-windows-x86_64.zip"])
        }
        ("zed-remote-server", "windows", "aarch64") => {
            Some(&["zed-remote-server-windows-aarch64.zip"])
        }
        _ => None,
    }
}

pub fn find_artifact(search_dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    for directory in search_dirs {
        if !directory.is_dir() {
            continue;
        }
        for name in names {
            let direct = directory.join(name);
            if direct.is_file() {
                return Some(direct);
            }
            if let Some(path) = find_named_newest(directory, name) {
                return Some(path);
            }
        }
    }
    None
}

fn find_named_newest(root: &Path, name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.file_name().and_then(|file_name| file_name.to_str()) != Some(name) {
                continue;
            }
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            match &newest {
                Some((existing, _)) if modified <= *existing => {}
                _ => newest = Some((modified, path)),
            }
        }
    }
    newest.map(|(_, path)| path)
}

async fn log_request(request: Request<Body>, next: axum::middleware::Next) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let response = next.run(request).await;
    eprintln!("{method} {uri} -> {}", response.status());
    response
}

async fn healthz() -> &'static str {
    "ok\n"
}

#[derive(Debug, Deserialize)]
struct AssetQuery {
    #[serde(default = "default_asset")]
    asset: String,
    #[serde(default)]
    os: String,
    #[serde(default)]
    arch: String,
}

fn default_asset() -> String {
    "zed".to_string()
}

async fn release_asset(
    State(state): State<Arc<AppState>>,
    PathParams((_channel, _version)): PathParams<(String, String)>,
    Query(query): Query<AssetQuery>,
    headers: HeaderMap,
) -> Response {
    let Some(names) = asset_file_names(&query.asset, &query.os, &query.arch) else {
        if asset_kind_known(&query.asset) {
            return json_error(
                StatusCode::NOT_FOUND,
                format!(
                    "no artifact mapping for os={} arch={}",
                    query.os, query.arch
                ),
            );
        }
        return json_error(
            StatusCode::NOT_FOUND,
            format!("unsupported asset {}", query.asset),
        );
    };
    let Some(path) = find_artifact(&state.search_dirs, names) else {
        return json_error(
            StatusCode::NOT_FOUND,
            format!("missing {}", names.join(" or ")),
        );
    };
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return json_error(StatusCode::NOT_FOUND, "artifact has no file name".into());
    };
    let origin = public_origin(
        state.public_url.as_deref(),
        header_str(&headers, "x-forwarded-proto"),
        header_str(&headers, "x-forwarded-host"),
        header_str(&headers, HOST.as_str()),
    );
    let payload = json!({
        "version": state.version,
        "url": format!("{origin}/files/{file_name}"),
    });
    json_response(StatusCode::OK, payload)
}

fn asset_kind_known(asset: &str) -> bool {
    asset == "zed" || asset == "zed-remote-server"
}

async fn serve_file(
    State(state): State<Arc<AppState>>,
    PathParams(name): PathParams<String>,
) -> Response {
    let Some(safe_name) = safe_file_name(&name) else {
        return text_error(StatusCode::NOT_FOUND, "not found\n");
    };
    let Some(path) = find_artifact(&state.search_dirs, &[safe_name]) else {
        return text_error(StatusCode::NOT_FOUND, "not found\n");
    };
    let file = match tokio::fs::File::open(&path).await {
        Ok(file) => file,
        Err(error) => {
            return text_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to open file: {error}\n"),
            );
        }
    };
    let content_length = match file.metadata().await {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            return text_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to stat file: {error}\n"),
            );
        }
    };
    let stream = ReaderStream::new(file);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_length.to_string()) {
        response.headers_mut().insert(CONTENT_LENGTH, value);
    }
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

fn safe_file_name(name: &str) -> Option<&str> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains('\0') {
        return None;
    }
    let file_name = Path::new(name).file_name()?.to_str()?;
    if file_name != name || file_name == "." || file_name == ".." {
        return None;
    }
    Some(file_name)
}

async fn proxy_extensions(State(state): State<Arc<AppState>>, request: Request<Body>) -> Response {
    let mut upstream = state.extensions_upstream.clone();
    upstream.set_path(request.uri().path());
    upstream.set_query(request.uri().query());

    let mut builder = state.http_client.request(Method::GET, upstream);
    for (name, value) in request.headers() {
        if FORWARDED_REQUEST_SKIP
            .iter()
            .any(|skipped| name.as_str().eq_ignore_ascii_case(skipped))
        {
            continue;
        }
        builder = builder.header(name, value);
    }

    let upstream_response = match builder.send().await {
        Ok(response) => response,
        Err(error) => {
            return text_error(
                StatusCode::BAD_GATEWAY,
                format!("extensions upstream error: {error}\n"),
            );
        }
    };

    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response_builder = Response::builder().status(status);
    let mut has_content_type = false;
    for (name, value) in upstream_response.headers() {
        if !FORWARDED_RESPONSE_ALLOW
            .iter()
            .any(|allowed| name.as_str().eq_ignore_ascii_case(allowed))
        {
            continue;
        }
        if name.as_str().eq_ignore_ascii_case("content-type") {
            has_content_type = true;
        }
        if let (Ok(header_name), Ok(header_value)) = (
            HeaderName::from_bytes(name.as_ref()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            if let Some(headers) = response_builder.headers_mut() {
                headers.append(header_name, header_value);
            }
        }
    }
    if !has_content_type {
        if let Some(headers) = response_builder.headers_mut() {
            headers.insert(
                CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
        }
    }

    let stream = upstream_response.bytes_stream();
    match response_builder.body(Body::from_stream(stream)) {
        Ok(response) => response,
        Err(error) => text_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to build proxy response: {error}\n"),
        ),
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn json_response(status: StatusCode, payload: serde_json::Value) -> Response {
    let body = payload.to_string();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
}

fn json_error(status: StatusCode, message: String) -> Response {
    json_response(status, json!({ "error": message }))
}

fn text_error(status: StatusCode, message: impl Into<String>) -> Response {
    let mut response = message.into().into_response();
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use std::io::Write;
    use tower::ServiceExt;

    fn test_state(search_dirs: Vec<PathBuf>, public_url: Option<&str>) -> Arc<AppState> {
        Arc::new(
            AppState::new(
                search_dirs,
                "1.22.1".into(),
                public_url.map(str::to_string),
                Url::parse("https://api.zed.dev").expect("upstream url"),
            )
            .expect("state"),
        )
    }

    async fn call(app: Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = app.oneshot(request).await.expect("response");
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    fn write_artifact(directory: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = directory.join(name);
        let mut file = std::fs::File::create(&path).expect("create artifact");
        file.write_all(contents).expect("write artifact");
        path
    }

    #[test]
    fn public_origin_prefers_configured_url() {
        assert_eq!(
            public_origin(
                Some("https://updates.burton.dev/"),
                Some("http"),
                Some("127.0.0.1:4180"),
                Some("localhost"),
            ),
            "https://updates.burton.dev"
        );
    }

    #[test]
    fn public_origin_uses_forwarded_headers() {
        assert_eq!(
            public_origin(
                None,
                Some("https, http"),
                Some("updates.burton.dev"),
                Some("127.0.0.1:4180"),
            ),
            "https://updates.burton.dev"
        );
    }

    #[test]
    fn public_origin_falls_back_to_host() {
        assert_eq!(
            public_origin(None, None, None, Some("127.0.0.1:4180")),
            "http://127.0.0.1:4180"
        );
    }

    #[test]
    fn asset_map_covers_burton_installers() {
        assert_eq!(
            asset_file_names("zed", "linux", "x86_64"),
            Some(&["burton-linux-x86_64.tar.gz"][..])
        );
        assert!(asset_file_names("zed", "plan9", "x86_64").is_none());
        assert!(asset_file_names("theme", "linux", "x86_64").is_none());
    }

    #[test]
    fn crate_version_reads_zed_package() {
        let version = crate_version(&repo_root()).expect("version");
        assert!(
            version.chars().next().is_some_and(|ch| ch.is_ascii_digit()),
            "unexpected version {version}"
        );
    }

    #[tokio::test]
    async fn healthz_ok() {
        let app = router(test_state(vec![], None));
        let (status, _, body) = call(
            app,
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, b"ok\n");
    }

    #[tokio::test]
    async fn release_asset_uses_public_url() {
        let directory = tempfile::tempdir().expect("tempdir");
        write_artifact(directory.path(), "burton-linux-x86_64.tar.gz", b"archive");
        let app = router(test_state(
            vec![directory.path().to_path_buf()],
            Some("https://updates.burton.dev"),
        ));
        let (status, headers, body) = call(
            app,
            Request::builder()
                .uri("/releases/stable/latest/asset?asset=zed&os=linux&arch=x86_64")
                .header("host", "127.0.0.1:4180")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload["version"], "1.22.1");
        assert_eq!(
            payload["url"],
            "https://updates.burton.dev/files/burton-linux-x86_64.tar.gz"
        );
    }

    #[tokio::test]
    async fn release_asset_uses_forwarded_host() {
        let directory = tempfile::tempdir().expect("tempdir");
        write_artifact(directory.path(), "burton-linux-x86_64.tar.gz", b"archive");
        let app = router(test_state(vec![directory.path().to_path_buf()], None));
        let (status, _, body) = call(
            app,
            Request::builder()
                .uri("/releases/stable/latest/asset?asset=zed&os=linux&arch=x86_64")
                .header("host", "127.0.0.1:4180")
                .header("x-forwarded-proto", "https")
                .header("x-forwarded-host", "updates.burton.dev")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            payload["url"],
            "https://updates.burton.dev/files/burton-linux-x86_64.tar.gz"
        );
    }

    #[tokio::test]
    async fn release_asset_missing_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        let app = router(test_state(vec![directory.path().to_path_buf()], None));
        let (status, _, body) = call(
            app,
            Request::builder()
                .uri("/releases/stable/latest/asset?asset=zed&os=linux&arch=x86_64")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(payload["error"].as_str().unwrap_or("").contains("missing"));
    }

    #[tokio::test]
    async fn release_asset_unsupported() {
        let app = router(test_state(vec![], None));
        let (status, _, body) = call(
            app,
            Request::builder()
                .uri("/releases/stable/latest/asset?asset=theme&os=linux&arch=x86_64")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert!(
            payload["error"]
                .as_str()
                .unwrap_or("")
                .contains("unsupported asset")
        );
    }

    #[tokio::test]
    async fn files_serves_installer() {
        let directory = tempfile::tempdir().expect("tempdir");
        write_artifact(directory.path(), "burton-linux-x86_64.tar.gz", b"archive");
        let app = router(test_state(vec![directory.path().to_path_buf()], None));
        let (status, headers, body) = call(
            app,
            Request::builder()
                .uri("/files/burton-linux-x86_64.tar.gz")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/octet-stream")
        );
        assert_eq!(body, b"archive");
    }

    #[tokio::test]
    async fn files_rejects_path_traversal() {
        let directory = tempfile::tempdir().expect("tempdir");
        let app = router(test_state(vec![directory.path().to_path_buf()], None));
        let (status, _, _) = call(
            app,
            Request::builder()
                .uri("/files/..%2Fsecret")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn extensions_proxy_follows_redirect_without_location() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let address = listener.local_addr().expect("local addr");
        let mock = Router::new()
            .route(
                "/extensions/html/download",
                get(|| async { axum::response::Redirect::temporary("/blob/html.tar.gz") }),
            )
            .route(
                "/blob/html.tar.gz",
                get(|| async {
                    (
                        [(CONTENT_TYPE, "application/gzip")],
                        Body::from(&b"GZIPDATA"[..]),
                    )
                }),
            );
        tokio::spawn(async move {
            axum::serve(listener, mock).await.expect("mock server");
        });

        let state = Arc::new(
            AppState::new(
                vec![],
                "1.22.1".into(),
                None,
                Url::parse(&format!("http://{address}")).expect("url"),
            )
            .expect("state"),
        );
        let app = router(state);
        let (status, headers, body) = call(
            app,
            Request::builder()
                .uri("/extensions/html/download")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers.get("location").is_none());
        assert_eq!(
            headers
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/gzip")
        );
        assert_eq!(body, b"GZIPDATA");
    }

    #[tokio::test]
    async fn extensions_proxy_passes_catalog_json() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock");
        let address = listener.local_addr().expect("local addr");
        let catalog = json!({"data": [{"id": "html", "name": "HTML"}]});
        let mock = Router::new().route(
            "/extensions",
            get(|| async { axum::Json(json!({"data": [{"id": "html", "name": "HTML"}]})) }),
        );
        tokio::spawn(async move {
            axum::serve(listener, mock).await.expect("mock server");
        });

        let state = Arc::new(
            AppState::new(
                vec![],
                "1.22.1".into(),
                None,
                Url::parse(&format!("http://{address}")).expect("url"),
            )
            .expect("state"),
        );
        let app = router(state);
        let (status, headers, body) = call(
            app,
            Request::builder()
                .uri("/extensions?filter=html")
                .body(Body::empty())
                .expect("request"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(headers.get("location").is_none());
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(payload, catalog);
    }
}
