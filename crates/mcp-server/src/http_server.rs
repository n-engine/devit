use std::{convert::Infallible, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    extract::Extension,
    http::{
        header::{AUTHORIZATION, CONTENT_TYPE, HOST},
        HeaderMap, HeaderValue, Method, StatusCode,
    },
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_stream::{
    wrappers::{errors::BroadcastStreamRecvError, BroadcastStream},
    StreamExt,
};
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};

use futures_core::Stream;
use futures_util::stream::once;

use crate::{
    auth::AuthManager,
    transport::{HttpAuthConfig, HttpCorsConfig, HttpTransportConfig},
    McpServer, NotificationHub,
};

#[derive(Clone)]
struct HttpState {
    inner: Arc<HttpStateInner>,
}

struct HttpStateInner {
    server: Arc<McpServer>,
    notifier: NotificationHub,
    auth: Option<AuthManager>,
    base_url_override: Option<String>,
    query_suffix: String,
}

impl HttpState {
    fn new(
        server: Arc<McpServer>,
        auth_config: Option<HttpAuthConfig>,
        base_url_override: Option<String>,
        query_suffix: String,
    ) -> Self {
        let auth = auth_config.map(|cfg| AuthManager::new(cfg.tokens));
        let notifier = server.notifier();
        Self {
            inner: Arc::new(HttpStateInner {
                server,
                notifier,
                auth,
                base_url_override,
                query_suffix,
            }),
        }
    }

    fn ensure_authorized(&self, headers: &HeaderMap) -> Result<(), ApiError> {
        if let Some(auth) = &self.inner.auth {
            if auth.validate(headers.get(AUTHORIZATION)) {
                return Ok(());
            }

            return Err(ApiError::Unauthorized);
        }

        Ok(())
    }

    fn server(&self) -> &Arc<McpServer> {
        &self.inner.server
    }

    fn notifier(&self) -> NotificationHub {
        self.inner.notifier.clone()
    }

    fn base_url_override(&self) -> Option<String> {
        self.inner.base_url_override.clone()
    }

    fn query_suffix(&self) -> &str {
        &self.inner.query_suffix
    }
}

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    Internal(anyhow::Error),
}

impl ApiError {
    fn internal<E: Into<anyhow::Error>>(err: E) -> Self {
        Self::Internal(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(json!({ "error": "unauthorized" })),
            )
                .into_response(),
            ApiError::Internal(err) => {
                tracing::error!("HTTP transport error: {:#}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "internal server error" })),
                )
                    .into_response()
            }
        }
    }
}

pub async fn run_http_transport(server: Arc<McpServer>, config: HttpTransportConfig) -> Result<()> {
    let HttpTransportConfig {
        host,
        port,
        sse_enabled,
        auth,
        cors,
    } = config;

    let base_url_override = std::env::var("MCP_HTTP_BASE_URL").ok();
    let query_suffix = std::env::var("MCP_HTTP_URL_SUFFIX").unwrap_or_default();

    let shared_state = HttpState::new(server.clone(), auth, base_url_override, query_suffix);

    let mut router = Router::new()
        .route("/.well-known/mcp.json", get(manifest))
        .route("/message", post(handle_message))
        .route("/health", get(health));

    if sse_enabled {
        router = router.route("/sse", get(events));
    }

    let router = router
        .layer(Extension(shared_state))
        .layer(build_cors_layer(cors.as_ref()))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .with_context(|| format!("Invalid bind address {host}:{port}"))?;

    tracing::info!("MCP HTTP server listening on {addr}");

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind MCP HTTP server to {addr}"))?;

    let make_service = axum::Router::into_make_service(router);

    axum::serve(listener, make_service)
        .await
        .context("HTTP transport encountered an unrecoverable error")?;

    Ok(())
}

async fn handle_message(
    Extension(state): Extension<HttpState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Result<Response, ApiError> {
    state.ensure_authorized(&headers)?;

    let result = state
        .server()
        .handle_jsonrpc(payload)
        .await
        .map_err(ApiError::internal)?;

    Ok(match result {
        Some(value) => (StatusCode::OK, Json(value)).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

async fn events(
    Extension(state): Extension<HttpState>,
    headers: HeaderMap,
) -> Result<Sse<Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>>, ApiError> {
    state.ensure_authorized(&headers)?;

    let receiver = state.notifier().subscribe();
    let initial_event =
        once(async { Ok::<Event, Infallible>(Event::default().event("ready").data("{}")) });

    let notifications = BroadcastStream::new(receiver).filter_map(|event| match event {
        Ok(value) => match serde_json::to_string(&value) {
            Ok(data) => Some(Ok(Event::default().event("notification").data(data))),
            Err(err) => {
                tracing::error!("Failed to serialize SSE payload: {err}");
                None
            }
        },
        Err(BroadcastStreamRecvError::Lagged(skipped)) => {
            tracing::warn!("SSE subscriber lagged by {skipped} messages");
            None
        }
    });

    let stream = futures_util::stream::select(initial_event, notifications);

    let keep_alive = KeepAlive::new()
        .interval(Duration::from_secs(15))
        .text("keep-alive");

    let boxed_stream: Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        Box::pin(stream);

    Ok(Sse::new(boxed_stream).keep_alive(keep_alive))
}

async fn manifest(Extension(state): Extension<HttpState>, headers: HeaderMap) -> impl IntoResponse {
    let base_url = state
        .base_url_override()
        .or_else(|| derive_base_url(&headers))
        .unwrap_or_else(|| "http://localhost:3001".to_string());

    let suffix = normalize_suffix(state.query_suffix());

    let message_url = format!("{}/message{}", base_url, suffix);
    let sse_url = format!("{}/sse{}", base_url, suffix);

    let manifest = json!({
        "protocolVersion": "2025-06-18",
        "transport": {
            "type": "http",
            "url": message_url,
            "sseUrl": sse_url,
        },
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {},
        },
        "serverInfo": {
            "name": "DevIt MCP Server",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Expose DevIt tools over MCP HTTP",
            "metadata": {
                "homepage": base_url,
            }
        }
    });

    Json(manifest)
}

async fn health() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "transport": "http",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

fn build_cors_layer(config: Option<&HttpCorsConfig>) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE]);

    if let Some(cors) = config {
        if !cors.allowed_origins.is_empty() {
            let origins: Vec<HeaderValue> = cors
                .allowed_origins
                .iter()
                .filter_map(|origin| origin.parse().ok())
                .collect();
            if !origins.is_empty() {
                return layer.allow_origin(AllowOrigin::list(origins));
            }
        }
    }

    layer
}

fn derive_base_url(headers: &HeaderMap) -> Option<String> {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())?;

    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .unwrap_or("http");

    Some(format!("{}://{}", scheme, host))
}

fn normalize_suffix(suffix: &str) -> String {
    if suffix.is_empty() {
        String::new()
    } else if suffix.starts_with('?') {
        suffix.to_string()
    } else {
        format!("?{}", suffix)
    }
}
