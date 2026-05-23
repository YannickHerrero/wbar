//! Tiny TCP control server. The running wbar instance listens on
//! 127.0.0.1:17128 and accepts one-line commands so external tools (AHK
//! scripts, glazewm keybinds, terminal one-liners) can toggle visibility or
//! switch themes without editing the config file.
//!
//! Protocol is intentionally trivial — one connection, one line, one
//! response. The CLI client (`wbar toggle`, `wbar set-theme Ink`, …) lives
//! in main.rs and just opens a stream, writes a line, reads the reply.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::str::FromStr;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use eframe::egui;

use crate::theme::Theme;

/// TCP port the control server binds on. Loopback-only; not configurable in
/// v1 — picked to avoid clashing with glazewm (6123) and common dev ports.
pub const PORT: u16 = 17128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcCommand {
    Toggle,
    Show,
    Hide,
    Quit,
    SetTheme(Theme),
}

impl IpcCommand {
    /// Parse a single line from the wire. Whitespace-trimmed, lowercased
    /// command word, optional space-separated argument. Errors return a
    /// human-readable message that the server echoes back to the client.
    pub fn parse(line: &str) -> Result<Self, String> {
        let line = line.trim();
        if line.is_empty() {
            return Err("empty command".into());
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();
        match cmd {
            "toggle" => Ok(Self::Toggle),
            "show" => Ok(Self::Show),
            "hide" => Ok(Self::Hide),
            "quit" => Ok(Self::Quit),
            "set-theme" => {
                if arg.is_empty() {
                    Err("set-theme requires a theme name".into())
                } else {
                    Theme::from_str(arg).map(Self::SetTheme)
                }
            }
            other => Err(format!("unknown command: {other:?}")),
        }
    }
}

/// Spawn the listener thread. Returns the receive half of the channel that
/// `WbarApp` drains each frame.
pub fn spawn(ctx: egui::Context) -> Result<Receiver<IpcCommand>> {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), PORT);
    let listener = TcpListener::bind(addr).with_context(|| format!("binding {addr}"))?;
    tracing::info!(%addr, "ipc listener bound");

    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("wbar-ipc".into())
        .spawn(move || {
            for conn in listener.incoming() {
                match conn {
                    Ok(stream) => {
                        if let Err(err) = handle_connection(stream, &tx, &ctx) {
                            tracing::debug!(?err, "ipc connection error");
                        }
                    }
                    Err(err) => tracing::warn!(?err, "ipc accept failed"),
                }
            }
        })
        .context("spawning ipc thread")?;
    Ok(rx)
}

fn handle_connection(
    mut stream: TcpStream,
    tx: &mpsc::Sender<IpcCommand>,
    ctx: &egui::Context,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let response = match IpcCommand::parse(&line) {
        Ok(cmd) => {
            tracing::info!(?cmd, "ipc command received");
            let _ = tx.send(cmd);
            ctx.request_repaint();
            "ok\n".to_string()
        }
        Err(err) => {
            tracing::warn!(line = %line.trim(), %err, "ipc rejected");
            format!("error: {err}\n")
        }
    };
    let _ = stream.write_all(response.as_bytes());
    Ok(())
}

/// Send a single command to the running wbar instance. Used by the CLI
/// client mode in `main.rs`. Returns the server's reply line.
pub fn send(command: &str) -> Result<String> {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), PORT);
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))
        .with_context(|| format!("connecting to {addr} (is wbar running?)"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    writeln!(stream, "{command}").context("writing ipc command")?;
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply).context("reading ipc reply")?;
    Ok(reply.trim().to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // expect-noise in test code adds no signal
mod tests {
    use super::*;

    #[test]
    fn parses_simple_commands() {
        assert_eq!(IpcCommand::parse("toggle").unwrap(), IpcCommand::Toggle);
        assert_eq!(IpcCommand::parse("show").unwrap(), IpcCommand::Show);
        assert_eq!(IpcCommand::parse("hide").unwrap(), IpcCommand::Hide);
        assert_eq!(IpcCommand::parse("quit").unwrap(), IpcCommand::Quit);
    }

    #[test]
    fn parses_set_theme() {
        assert_eq!(
            IpcCommand::parse("set-theme Stone").unwrap(),
            IpcCommand::SetTheme(Theme::Stone),
        );
        // Case-insensitive
        assert_eq!(
            IpcCommand::parse("set-theme ink").unwrap(),
            IpcCommand::SetTheme(Theme::Ink),
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(IpcCommand::parse("").is_err());
        assert!(IpcCommand::parse("nope").is_err());
        assert!(IpcCommand::parse("set-theme").is_err());
        assert!(IpcCommand::parse("set-theme Mocha").is_err());
    }
}
