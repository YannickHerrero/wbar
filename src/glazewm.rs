//! Tiny client for the GlazeWM IPC websocket. Connects to ws://127.0.0.1:6123,
//! subscribes to the workspace/focus events we care about, queries initial
//! state, and surfaces JSON envelopes to the rest of the app.
//!
//! This commit just wires up the connection and logs traffic; the workspaces
//! widget consumes the data in the next commit, and the smarter reconnect
//! backoff lands the commit after that.

use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use eframe::egui;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;

const GLAZEWM_WS_URL: &str = "ws://127.0.0.1:6123";

/// Subscribe to the minimum set of events that move workspaces around.
const SUBSCRIBE_CMD: &str = concat!(
    "sub -e workspace_activated workspace_deactivated workspace_updated ",
    "focused_container_moved focus_changed",
);
const QUERY_WORKSPACES_CMD: &str = "query workspaces";

/// Spawn a background thread that maintains a connection to glazewm and logs
/// events. The thread will be torn down when the process exits; we keep no
/// JoinHandle because there's nothing meaningful to wait on.
pub fn spawn(ctx: egui::Context) {
    thread::Builder::new()
        .name("glazewm-ipc".into())
        .spawn(move || run(ctx))
        .expect("spawning glazewm IPC thread");
}

fn run(_ctx: egui::Context) {
    loop {
        match session() {
            Ok(()) => tracing::info!("glazewm session ended cleanly"),
            Err(err) => tracing::warn!(error = ?err, "glazewm session error"),
        }
        // Naive sleep — commit 19 replaces this with exponential backoff.
        thread::sleep(Duration::from_secs(1));
    }
}

fn session() -> Result<()> {
    let request = GLAZEWM_WS_URL
        .into_client_request()
        .context("building ws client request")?;
    let (mut socket, _response) = tungstenite::client::client(request, connect_tcp()?)
        .map_err(|e| anyhow::anyhow!("websocket handshake failed: {e}"))?;
    tracing::info!(url = GLAZEWM_WS_URL, "glazewm connected");

    socket.send(Message::text(SUBSCRIBE_CMD))?;
    socket.send(Message::text(QUERY_WORKSPACES_CMD))?;

    loop {
        match socket.read()? {
            Message::Text(text) => tracing::debug!(payload = %text, "glazewm message"),
            Message::Binary(_) => tracing::debug!("glazewm binary frame (ignored)"),
            Message::Ping(p) => socket.send(Message::Pong(p))?,
            Message::Pong(_) => {}
            Message::Close(_) => {
                tracing::info!("glazewm server closed connection");
                return Ok(());
            }
            Message::Frame(_) => {}
        }
    }
}

fn connect_tcp() -> Result<MaybeTlsStream<TcpStream>> {
    let stream = TcpStream::connect(("127.0.0.1", 6123)).context("connecting to glazewm")?;
    stream.set_nodelay(true).ok();
    Ok(MaybeTlsStream::Plain(stream))
}
