//! HTTP server setup with axum.

use crate::routes;
use crate::state::AppState;
use axum::Router;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::trace::TraceLayer;
use tracing::info;

/// Build the axum router with all routes and middleware.
fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        // Phase 2a: health and shutdown
        .merge(routes::health::router())
        .merge(routes::shutdown::router())
        // Phase 2b: embeddings and rag search
        .merge(routes::embeddings::router())
        .merge(routes::rag::router())
        // Phase 2c: respond
        .merge(routes::respond::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Start the HTTP server and run until shutdown signal.
pub async fn serve(state: Arc<AppState>) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("{}:{}", state.config.server.host, state.config.server.port)
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid bind address: {}", e))?;

    let router = build_router(state.clone());

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind to {}: {}", addr, e);
            if e.kind() == std::io::ErrorKind::AddrInUse {
                tracing::error!(
                    "Port {} is already in use. Another instance may be running, \
                     or another service (like Ollama) is using this port.",
                    addr.port()
                );
            }
            return Err(anyhow::anyhow!("Failed to bind to {}: {}", addr, e));
        }
    };

    info!("Listening on http://{}", addr);

    // Set up graceful shutdown
    let shutdown_signal = state.shutdown.clone();
    let graceful_shutdown = async move {
        shutdown_signal.notified().await;
        info!("Shutdown signal received, draining connections...");
        // Allow 5 seconds for in-flight requests to drain
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    };

    axum::serve(listener, router)
        .with_graceful_shutdown(graceful_shutdown)
        .await?;

    info!("Server stopped cleanly");
    Ok(())
}
