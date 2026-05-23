//! Tiny client for the GlazeWM IPC websocket. Connects to ws://127.0.0.1:6123,
//! subscribes to the workspace/focus events we care about, queries workspace
//! state, and surfaces a parsed snapshot to widgets via a shared `Arc<RwLock>`.
//!
//! GlazeWM's IPC format is a JSON envelope. Both event subscriptions and
//! query responses come back through the same socket; we treat any event as
//! "something might have changed" and just re-issue `query workspaces` to
//! re-snapshot, since incremental patching isn't worth the complexity.

use std::net::TcpStream;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use eframe::egui;
use serde::Deserialize;
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;

const GLAZEWM_WS_URL: &str = "ws://127.0.0.1:6123";

const SUBSCRIBE_CMD: &str = concat!(
    "sub -e workspace_activated workspace_deactivated workspace_updated ",
    "focused_container_moved focus_changed",
);
const QUERY_WORKSPACES_CMD: &str = "query workspaces";

/// What widgets see. Cheap to clone — small Vec of small structs.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceState {
    pub workspaces: Vec<WorkspaceInfo>,
    pub connected: bool,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    /// The workspace's internal name; kept around for future widget options
    /// (e.g. rendering name vs. display_name) and tooltip hover.
    #[allow(dead_code)]
    pub name: String,
    pub display_name: String,
    pub focused: bool,
}

/// Shared handle. Clone is cheap; both halves point at the same `Arc<RwLock>`.
#[derive(Clone)]
pub struct GlazewmClient {
    state: Arc<RwLock<WorkspaceState>>,
}

impl GlazewmClient {
    pub fn spawn(ctx: egui::Context) -> Self {
        let state = Arc::new(RwLock::new(WorkspaceState::default()));
        let inner = state.clone();
        thread::Builder::new()
            .name("glazewm-ipc".into())
            .spawn(move || run(ctx, inner))
            .expect("spawning glazewm IPC thread");
        Self { state }
    }

    /// Snapshot the current state. Holds the read lock for the duration of the
    /// clone, which is brief.
    pub fn snapshot(&self) -> WorkspaceState {
        self.state.read().map(|s| s.clone()).unwrap_or_default()
    }
}

const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(10);
/// If a session stayed up at least this long, treat it as "real" and reset
/// backoff. Otherwise glazewm crash-looping would peg us at the minimum delay.
const BACKOFF_RESET_AFTER: Duration = Duration::from_secs(5);

fn run(ctx: egui::Context, state: Arc<RwLock<WorkspaceState>>) {
    let mut backoff = BACKOFF_MIN;
    // Track whether we've ever successfully connected, so the log path can
    // distinguish "glazewm just disappeared" (worth a warn) from "glazewm
    // isn't running and probably won't be" (worth one info, then silence).
    let mut ever_connected = false;
    let mut unreachable_logged = false;
    loop {
        let connected_at = Instant::now();
        let result = session(&ctx, &state);
        let stayed_up_for = connected_at.elapsed();
        let connected_this_round = stayed_up_for >= Duration::from_millis(50)
            && state.read().map(|s| s.connected).unwrap_or(false);

        match result {
            Ok(()) => {
                tracing::info!("glazewm session ended cleanly");
                ever_connected = true;
                unreachable_logged = false;
            }
            Err(err) if ever_connected => {
                // We had a working session and lost it — that's worth a warn.
                tracing::warn!(error = ?err, "glazewm session dropped");
                unreachable_logged = false;
            }
            Err(err) if !unreachable_logged => {
                // First time we couldn't reach glazewm. One quiet log, then
                // we go silent so this isn't spam when glazewm isn't running.
                tracing::info!(
                    error = %err,
                    "glazewm not reachable; the workspaces widget will stay disabled until it appears",
                );
                unreachable_logged = true;
            }
            Err(err) => {
                tracing::trace!(error = ?err, "glazewm still unreachable");
            }
        }

        if connected_this_round {
            ever_connected = true;
            unreachable_logged = false;
        }

        set_connected(&state, false);
        ctx.request_repaint();

        if stayed_up_for >= BACKOFF_RESET_AFTER {
            backoff = BACKOFF_MIN;
        }
        tracing::debug!(delay = ?backoff, "glazewm reconnect scheduled");
        thread::sleep(backoff);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

fn session(ctx: &egui::Context, state: &Arc<RwLock<WorkspaceState>>) -> Result<()> {
    let request = GLAZEWM_WS_URL
        .into_client_request()
        .context("building ws client request")?;
    let (mut socket, _response) = tungstenite::client::client(request, connect_tcp()?)
        .map_err(|e| anyhow::anyhow!("websocket handshake failed: {e}"))?;
    tracing::info!(url = GLAZEWM_WS_URL, "glazewm connected");
    set_connected(state, true);
    ctx.request_repaint();

    socket.send(Message::text(SUBSCRIBE_CMD))?;
    socket.send(Message::text(QUERY_WORKSPACES_CMD))?;

    loop {
        match socket.read()? {
            Message::Text(text) => handle_text(&text, ctx, state, &mut socket)?,
            Message::Ping(p) => socket.send(Message::Pong(p))?,
            Message::Close(_) => {
                tracing::info!("glazewm server closed connection");
                return Ok(());
            }
            Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}

fn handle_text(
    text: &str,
    ctx: &egui::Context,
    state: &Arc<RwLock<WorkspaceState>>,
    socket: &mut tungstenite::WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<()> {
    let envelope: Envelope = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(err) => {
            tracing::debug!(?err, "glazewm: unparseable envelope");
            return Ok(());
        }
    };

    match envelope.message_type.as_deref() {
        Some("client_response") => {
            if let Some(ws_list) = extract_workspaces(&envelope.data) {
                update_state(state, ws_list);
                ctx.request_repaint();
            }
        }
        Some("event_subscription") => {
            // Any change → re-query for a fresh snapshot.
            socket.send(Message::text(QUERY_WORKSPACES_CMD))?;
        }
        _ => {}
    }
    Ok(())
}

fn update_state(state: &Arc<RwLock<WorkspaceState>>, new_list: Vec<WorkspaceInfo>) {
    if let Ok(mut s) = state.write() {
        s.workspaces = new_list;
        s.connected = true;
    }
}

fn set_connected(state: &Arc<RwLock<WorkspaceState>>, connected: bool) {
    if let Ok(mut s) = state.write() {
        s.connected = connected;
    }
}

fn connect_tcp() -> Result<MaybeTlsStream<TcpStream>> {
    let stream = TcpStream::connect(("127.0.0.1", 6123)).context("connecting to glazewm")?;
    stream.set_nodelay(true).ok();
    Ok(MaybeTlsStream::Plain(stream))
}

// ---------------------------------------------------------------------------
// JSON shapes
//
// GlazeWM's IPC schema isn't formally versioned in our consumer; if the server
// adds fields or changes case, we want to fail soft (log debug, keep stale
// state) rather than crash. That's why everything below uses #[serde(default)]
// and Option, and why we re-query rather than apply patches.

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "messageType")]
    message_type: Option<String>,
    data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceJson {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "hasFocus")]
    has_focus: bool,
}

fn extract_workspaces(data: &Option<serde_json::Value>) -> Option<Vec<WorkspaceInfo>> {
    let data = data.as_ref()?;
    let workspaces = data
        .get("workspaces")
        .or_else(|| data.pointer("/workspaces"))?;
    let parsed: Vec<WorkspaceJson> = serde_json::from_value(workspaces.clone()).ok()?;
    Some(
        parsed
            .into_iter()
            .map(|w| WorkspaceInfo {
                display_name: w.display_name.clone().unwrap_or_else(|| w.name.clone()),
                name: w.name,
                focused: w.has_focus,
            })
            .collect(),
    )
}
