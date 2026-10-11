mod handler;
mod queue;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::sync::Semaphore;

use crate::error::{Result, TeamsError};
use crate::output;

/// Connections served at once; further ones wait in the accept backlog.
const MAX_CONNECTIONS: usize = 64;
/// Time allowed to send request headers, including on an idle keep-alive
/// connection, so a stalled client releases its connection slot.
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
/// Time allowed on Ctrl+C to write output already accepted.
const SHUTDOWN_FLUSH: Duration = Duration::from_secs(2);

/// Run the webhook listener HTTP server on `bind:port`.
///
/// The server handles:
/// - GET `/` — health check (200 OK)
/// - POST with `?validationToken=xxx` — Graph subscription validation (echoes token)
/// - POST with JSON body — change notifications, printed as NDJSON to stdout
///   by a writer thread; 202 once queued, 503 when the queue is full
///
/// With `client_state`, notifications carrying another or no `clientState`
/// are dropped.
///
/// Note: Microsoft Graph webhooks require HTTPS. Use a reverse proxy (e.g., ngrok)
/// in front of this listener for production use.
pub async fn run_listener(bind: IpAddr, port: u16, client_state: Option<String>) -> Result<()> {
    let addr = SocketAddr::new(bind, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| TeamsError::ConfigError(format!("cannot listen on {addr}: {e}")))?;
    let local = listener.local_addr().unwrap_or(addr);

    let queue = queue::Queue::new(queue::OUTPUT_BUDGET, output::write_stdout_line);
    let state = Arc::new(handler::State::new(client_state.as_deref(), queue));
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));

    eprintln!("Webhook listener started on http://{local}");
    if client_state.is_none() {
        eprintln!(
            "Warning: notifications are not authenticated; set --client-state to verify them"
        );
    }
    eprintln!("Press Ctrl+C to stop");

    loop {
        tokio::select! {
            (slot, accept) = async {
                let slot = slots.clone().acquire_owned().await;
                (slot, listener.accept().await)
            } => {
                let Ok(slot) = slot else { continue };
                match accept {
                    Ok((stream, _remote)) => {
                        let io = TokioIo::new(stream);
                        let state = state.clone();
                        tokio::spawn(async move {
                            let _slot = slot;
                            let service = service_fn(move |req| {
                                handler::handle_request(req, state.clone())
                            });
                            let _ = hyper::server::conn::http1::Builder::new()
                                .timer(TokioTimer::new())
                                .header_read_timeout(HEADER_TIMEOUT)
                                .serve_connection(io, service)
                                .await;
                        });
                    }
                    Err(e) => {
                        tracing::warn!("Failed to accept connection: {e}");
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nShutting down webhook listener");
                state.queue().flush(SHUTDOWN_FLUSH).await;
                return Ok(());
            }
        }
    }
}
