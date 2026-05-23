# wbar

A minimalist status bar for Windows, designed to pair with [GlazeWM](https://github.com/glzr-io/glazewm).

Inspired by [zebar](https://github.com/glzr-io/zebar) but without the webview / React build pipeline — just a single TOML config file, a small set of built-in widgets, and five named themes out of the box.

## Status

Early development. Pre-1.0.

## Build

```
cargo build --release --target x86_64-pc-windows-msvc
```

The binary lands in `target\x86_64-pc-windows-msvc\release\wbar.exe`.

## Config

wbar reads `%APPDATA%\wbar\config.toml`. If the file doesn't exist, an embedded default is used. See [`examples/config.toml`](examples/config.toml) for a fully-annotated template.

Edit and save the config; the bar refreshes live — no restart required for theme, palette, font, widget format strings, or layout. Changes to `bar.position` / `bar.height` re-register the Windows AppBar transparently.

## Themes

Pick one in `config.toml`:

- **Paper** — warm cream paper, ink black text, terracotta accent
- **Stone** — cool gray, blue accent
- **Sage** — muted greens
- **Clay** — warm earth tones
- **Ink** — black background, off-white text (only dark theme)

Override any individual colour via the optional `[palette]` table (hex strings), or tweak spacing/radius/font sizes via `[tokens]`. Custom themes are pure config — no recompile.

## Widgets

- `glazewm` — workspace indicators, focused workspace highlighted
- `clock` — formatted local time (chrono strftime)
- `sysinfo` — CPU usage, RAM, or network throughput (rx/tx)
- `command` — run an arbitrary shell command on an interval, show its stdout

Each widget is display-only in v1 — no click interactions.

## How it integrates with Windows

- The window is borderless, always-on-top, and hidden from the taskbar.
- It registers itself as a Windows AppBar via `SHAppBarMessage`, so maximised windows stop short of the bar instead of going under it.
- AppBar registration is automatically removed when the bar exits.

## Hot reload

Config changes are detected by watching the parent directory of `config.toml` (robust to the temp-file-rename pattern most editors use). A fresh parse runs in-process and the bar restyles on the next frame.

## License

MIT
