//! Streamable HTTP on a loopback address, for clients that connect to a
//! running server. Every request carries the bearer token; the `Host`
//! header must name the loopback; a browser's `Origin` is refused; bodies
//! are capped. The address and token are written to a connection file only
//! its owner can read, removed when the server stops.
use super::server::{NeptuneVideoServer, Shared};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tokio_util::sync::CancellationToken;

pub const DEFAULT_ADDRESS: &str = "127.0.0.1:47420";
/// The largest request body.
const MOST_BODY: usize = 4 * 1024 * 1024;

/// A token of 244 random bits from the system's generator.
fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

async fn authorize(
    State(token): State<Arc<String>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if given.is_empty() || !editor_agent::same_token(given.trim(), &token) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "A bearer token is required",
        )
            .into_response();
    }
    next.run(request).await
}

/// The connection file, in `folder`: where clients and scripts find the
/// address and token of the running server.
fn connection_file(folder: &std::path::Path) -> PathBuf {
    folder.join(format!("{}.json", std::process::id()))
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// Serves until interrupted.
/// `connections` is the folder the connection file goes in.
pub async fn serve(
    shared: Arc<Shared>,
    address: &str,
    token: Option<String>,
    connections: Option<PathBuf>,
) -> Result<(), String> {
    let address: SocketAddr = address
        .parse()
        .map_err(|_| format!("{address} is not an IP address and port (use 127.0.0.1:PORT)"))?;
    if !address.ip().is_loopback() {
        return Err(format!(
            "{address} is not a loopback address; neptune-video-mcp serves this computer only"
        ));
    }
    let given = token.is_some();
    let token = Arc::new(token.unwrap_or_else(random_token));
    let stop = CancellationToken::new();
    let mut config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(["localhost", "127.0.0.1", "::1", "[::1]"])
        .enforce_origin_validation();
    config.max_request_body_bytes = MOST_BODY;
    config.cancellation_token = stop.child_token();
    let factory = {
        let shared = shared.clone();
        move || Ok(NeptuneVideoServer::new(shared.clone()))
    };
    let service: StreamableHttpService<NeptuneVideoServer, LocalSessionManager> =
        StreamableHttpService::new(factory, Default::default(), config);
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(token.clone(), authorize));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("cannot listen on {address}: {error}"))?;
    let bound = listener.local_addr().map_err(|error| error.to_string())?;
    let url = format!("http://{bound}/mcp");
    let file = connections.as_deref().map(connection_file);
    if let Some(file) = &file {
        let body = serde_json::json!({
            "url": url,
            "token": *token,
            "pid": std::process::id(),
            "roots": shared.roots.list(),
        });
        match write_private(file, &serde_json::to_vec_pretty(&body).unwrap_or_default()) {
            Ok(()) => eprintln!("neptune-video-mcp: connection file {}", file.display()),
            Err(error) => {
                eprintln!("neptune-video-mcp: the connection file was not written: {error}")
            }
        }
    }
    eprintln!(
        "neptune-video-mcp: serving {url} ({})",
        if given {
            "token from NEPTUNE_VIDEO_MCP_TOKEN"
        } else {
            "token in the connection file"
        }
    );
    let served = axum::serve(listener, router)
        .with_graceful_shutdown({
            let stop = stop.clone();
            async move {
                shutdown_signal().await;
                stop.cancel();
            }
        })
        .await;
    if let Some(file) = &file {
        let _ = std::fs::remove_file(file);
    }
    served.map_err(|error| error.to_string())
}

/// Ctrl-C, and on Unix the termination and hang-up a client sends when it
/// stops its server.
pub(super) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut terminate), Ok(mut hang_up)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) else {
            let _ = tokio::signal::ctrl_c().await;
            return;
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
            _ = hang_up.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
