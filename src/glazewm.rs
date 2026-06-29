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

use crate::wake::Waker;

const GLAZEWM_WS_URL: &str = "ws://127.0.0.1:6123";

const SUBSCRIBE_CMD: &str = concat!(
    "sub -e workspace_activated workspace_deactivated workspace_updated ",
    "focused_container_moved focus_changed tiling_direction_changed",
);
const QUERY_MONITORS_CMD: &str = "query monitors";
const QUERY_TILING_DIRECTION_CMD: &str = "query tiling-direction";
const FOCUS_WORKSPACE_CMD_PREFIX: &str = "command focus --workspace ";

/// What widgets see. Cheap to clone — small Vecs of small structs.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceState {
    /// Workspaces grouped by the monitor they belong to, so a per-monitor bar
    /// can show its own display's workspaces.
    pub monitors: Vec<MonitorWorkspaces>,
    pub tiling_direction: Option<TilingDirection>,
    pub connected: bool,
}

/// One monitor and the workspaces assigned to it.
#[derive(Debug, Clone)]
pub struct MonitorWorkspaces {
    /// GDI device name (e.g. `\\.\DISPLAY1`), the join key to a Win32 monitor.
    pub device_name: String,
    /// Whether this is the globally-focused monitor.
    pub has_focus: bool,
    pub workspaces: Vec<WorkspaceInfo>,
}

/// Which monitor's workspaces a widget should show.
#[derive(Debug, Clone)]
pub enum MonitorTarget {
    /// The currently-focused monitor (used by the single-bar / fallback case).
    Focused,
    /// A specific monitor by GDI device name.
    Device(String),
}

impl WorkspaceState {
    /// Resolve the monitor a widget targets, if present in the current state.
    pub fn monitor_for(&self, target: &MonitorTarget) -> Option<&MonitorWorkspaces> {
        match target {
            MonitorTarget::Focused => self.monitors.iter().find(|m| m.has_focus),
            MonitorTarget::Device(name) => self.monitors.iter().find(|m| m.device_name == *name),
        }
    }
}

/// Direction in which the focused container will place a new tiling window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilingDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    /// The workspace's internal name; kept around for future widget options
    /// (e.g. rendering name vs. display_name) and tooltip hover.
    #[allow(dead_code)]
    pub name: String,
    pub display_name: String,
    /// Whether this workspace holds the global input focus.
    pub focused: bool,
    /// Whether this workspace is the one currently shown on its monitor.
    pub is_displayed: bool,
}

/// Shared handle. Clone is cheap; both halves point at the same `Arc<RwLock>`.
#[derive(Clone)]
pub struct GlazewmClient {
    state: Arc<RwLock<WorkspaceState>>,
}

impl GlazewmClient {
    pub fn spawn(ctx: egui::Context, waker: Waker) -> Self {
        let state = Arc::new(RwLock::new(WorkspaceState::default()));
        let inner = state.clone();
        thread::Builder::new()
            .name("glazewm-ipc".into())
            .spawn(move || run(ctx, waker, inner))
            .expect("spawning glazewm IPC thread");
        Self { state }
    }

    /// Snapshot the current state. Holds the read lock for the duration of the
    /// clone, which is brief.
    pub fn snapshot(&self) -> WorkspaceState {
        self.state.read().map(|s| s.clone()).unwrap_or_default()
    }

    /// Ask GlazeWM to focus the workspace with this internal name. The command
    /// is sent on a short-lived background connection so a click never blocks
    /// egui's render loop.
    pub fn focus_workspace(&self, workspace_name: &str) {
        let workspace_name = workspace_name.to_owned();
        thread::Builder::new()
            .name("glazewm-command".into())
            .spawn(move || {
                if let Err(err) = send_focus_workspace(&workspace_name) {
                    tracing::warn!(workspace = %workspace_name, error = ?err, "failed to focus glazewm workspace");
                }
            })
            .ok();
    }
}

const BACKOFF_MIN: Duration = Duration::from_millis(500);
const BACKOFF_MAX: Duration = Duration::from_secs(10);
/// If a session stayed up at least this long, treat it as "real" and reset
/// backoff. Otherwise glazewm crash-looping would peg us at the minimum delay.
const BACKOFF_RESET_AFTER: Duration = Duration::from_secs(5);

fn run(ctx: egui::Context, waker: Waker, state: Arc<RwLock<WorkspaceState>>) {
    let mut backoff = BACKOFF_MIN;
    let mut ever_connected = false;
    let mut unreachable_logged = false;
    loop {
        let connected_at = Instant::now();
        let result = session(&ctx, &waker, &state);
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
                tracing::warn!(error = ?err, "glazewm session dropped");
                unreachable_logged = false;
            }
            Err(err) if !unreachable_logged => {
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
        waker.wake();

        if stayed_up_for >= BACKOFF_RESET_AFTER {
            backoff = BACKOFF_MIN;
        }
        tracing::debug!(delay = ?backoff, "glazewm reconnect scheduled");
        thread::sleep(backoff);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

fn session(ctx: &egui::Context, waker: &Waker, state: &Arc<RwLock<WorkspaceState>>) -> Result<()> {
    let request = GLAZEWM_WS_URL
        .into_client_request()
        .context("building ws client request")?;
    let (mut socket, _response) = tungstenite::client::client(request, connect_tcp()?)
        .map_err(|e| anyhow::anyhow!("websocket handshake failed: {e}"))?;
    tracing::info!(url = GLAZEWM_WS_URL, "glazewm connected");
    set_connected(state, true);
    ctx.request_repaint();
    waker.wake();

    socket.send(Message::text(SUBSCRIBE_CMD))?;
    socket.send(Message::text(QUERY_MONITORS_CMD))?;
    socket.send(Message::text(QUERY_TILING_DIRECTION_CMD))?;

    loop {
        match socket.read()? {
            Message::Text(text) => handle_text(&text, ctx, waker, state, &mut socket)?,
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
    waker: &Waker,
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
            let mut changed = false;
            if let Some(monitors) = extract_monitors(&envelope.data) {
                update_monitors(state, monitors);
                changed = true;
            }
            if let Some(dir) = extract_tiling_direction(&envelope.data) {
                update_tiling_direction(state, Some(dir));
                changed = true;
            }
            if changed {
                ctx.request_repaint();
                waker.wake();
            }
        }
        Some("event_subscription") => {
            // Any subscribed event might change either workspace state or
            // the tiling direction (focus moves can change the parent's
            // direction). Re-issue both queries; the responses come back as
            // client_response messages handled above.
            socket.send(Message::text(QUERY_MONITORS_CMD))?;
            socket.send(Message::text(QUERY_TILING_DIRECTION_CMD))?;
        }
        _ => {}
    }
    Ok(())
}

fn update_monitors(state: &Arc<RwLock<WorkspaceState>>, monitors: Vec<MonitorWorkspaces>) {
    if let Ok(mut s) = state.write() {
        s.monitors = monitors;
        s.connected = true;
    }
}

fn update_tiling_direction(state: &Arc<RwLock<WorkspaceState>>, dir: Option<TilingDirection>) {
    if let Ok(mut s) = state.write() {
        s.tiling_direction = dir;
        s.connected = true;
    }
}

fn set_connected(state: &Arc<RwLock<WorkspaceState>>, connected: bool) {
    if let Ok(mut s) = state.write() {
        s.connected = connected;
    }
}

fn send_focus_workspace(workspace_name: &str) -> Result<()> {
    let request = GLAZEWM_WS_URL
        .into_client_request()
        .context("building ws client request")?;
    let (mut socket, _response) = tungstenite::client::client(request, connect_tcp()?)
        .map_err(|e| anyhow::anyhow!("websocket handshake failed: {e}"))?;
    let command = format!(
        "{FOCUS_WORKSPACE_CMD_PREFIX}{}",
        command_arg(workspace_name)
    );
    socket.send(Message::text(command))?;
    socket.close(None).ok();
    Ok(())
}

fn command_arg(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':' | '/'))
    {
        return value.to_owned();
    }

    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
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
struct MonitorJson {
    #[serde(default, rename = "deviceName")]
    device_name: Option<String>,
    #[serde(default, rename = "hasFocus")]
    has_focus: bool,
    /// A monitor's children are its workspaces.
    #[serde(default)]
    children: Vec<WorkspaceJson>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceJson {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default, rename = "hasFocus")]
    has_focus: bool,
    #[serde(default, rename = "isDisplayed")]
    is_displayed: bool,
}

fn extract_monitors(data: &Option<serde_json::Value>) -> Option<Vec<MonitorWorkspaces>> {
    let data = data.as_ref()?;
    let monitors = data.get("monitors")?;
    let parsed: Vec<MonitorJson> = serde_json::from_value(monitors.clone()).ok()?;
    Some(
        parsed
            .into_iter()
            .map(|m| MonitorWorkspaces {
                device_name: m.device_name.unwrap_or_default(),
                has_focus: m.has_focus,
                workspaces: m
                    .children
                    .into_iter()
                    .map(|w| {
                        // GlazeWM returns an empty displayName when none is
                        // configured; fall back to the workspace name so the
                        // pill isn't blank.
                        let display_name = match w.display_name {
                            Some(d) if !d.is_empty() => d,
                            _ => w.name.clone(),
                        };
                        WorkspaceInfo {
                            display_name,
                            name: w.name,
                            focused: w.has_focus,
                            is_displayed: w.is_displayed,
                        }
                    })
                    .collect(),
            })
            .collect(),
    )
}

fn extract_tiling_direction(data: &Option<serde_json::Value>) -> Option<TilingDirection> {
    let s = data.as_ref()?.get("tilingDirection")?.as_str()?;
    match s {
        "horizontal" => Some(TilingDirection::Horizontal),
        "vertical" => Some(TilingDirection::Vertical),
        _ => None,
    }
}
