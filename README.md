# wbar

A minimalist status bar for Windows, designed to pair with [GlazeWM](https://github.com/glzr-io/glazewm).

Single binary, no webview, no build pipeline — just one TOML config file, a handful of built-in widgets, and five named themes out of the box.

```
                                                                            
   workspaces                Mon 3 Jan 2:30 PM                 78%  42%  88%
                                                                            
```

- Borderless, always-on-top, registers as a Windows AppBar so maximised windows respect it
- TOML config file with hot-reload (no restart on edit)
- Five named themes — `Paper`, `Stone`, `Sage`, `Clay`, `Ink` — with per-field overrides
- Nerd Font icons (auto-discovered from your system font folder)
- System tray icon for show/hide/quit
- Terminal control: `wbar toggle`, `wbar set-theme Ink`, …

## Table of contents

- [Install](#install)
- [Run](#run)
- [Configuration](#configuration)
- [Widgets](#widgets)
- [Themes](#themes)
- [Customisation](#customisation)
- [Nerd Font icons](#nerd-font-icons)
- [Control from the terminal](#control-from-the-terminal)
- [Building from source](#building-from-source)
- [Contributing](#contributing)
- [License](#license)

## Install

Grab `wbar.exe` from the [latest release](https://github.com/yannickherrero/wbar/releases) and drop it anywhere on your PATH (or somewhere convenient like `%USERPROFILE%\Documents\apps\`).

Or build from source — see [Building from source](#building-from-source).

## Run

Just run the binary:

```powershell
wbar.exe
```

The bar appears at the top of your primary monitor and registers as a Windows AppBar so maximised windows stop short of it.

First run with no config writes the embedded default at `%APPDATA%\wbar\config.toml` style defaults (you'll only need to create the file if you want to change something).

## Configuration

wbar reads `%APPDATA%\wbar\config.toml`. If the file doesn't exist, an embedded default is used. Edit and save the config — the bar restyles live, no restart needed. Changes to `bar.position` / `bar.height` re-register the Windows AppBar transparently.

Annotated example ([`examples/config.toml`](examples/config.toml)):

```toml
[bar]
position = "top"        # "top" | "bottom"
height = 28

[font]
family = "Default"      # informational; egui's bundled monospace is used
size = 12.0

# One of: Paper | Stone | Sage | Clay | Ink
theme = "Paper"

# Which widgets go in which region. Each id references a [widgets.<id>] table.
[layout]
left   = ["workspaces"]
center = ["clock"]
# Right region renders right-to-left: the first id is rightmost on screen.
right  = ["battery", "cpu", "memory"]

[widgets.workspaces]
type = "glazewm"
show_empty = false

[widgets.clock]
type = "clock"
format = "%a %-d %b %-I:%M %p"   # chrono strftime
tick_seconds = 1

[widgets.memory]
type = "sysinfo"
metric = "ram"
icon = ""              # nf-fa-microchip
format = "{value:.0}%"
interval_seconds = 2

[widgets.cpu]
type = "sysinfo"
metric = "cpu"
icon = ""              # nf-oct-cpu
format = "{value:.0}%"
interval_seconds = 2
warn_above = 85.0       # text turns red above this %

[widgets.battery]
type = "sysinfo"
metric = "battery"
icon = ""              # nf-fa-battery_full  — on battery
charging_icon = "󰂄"     # nf-md-battery_charging — on AC
format = "{value:.0}%"
interval_seconds = 10
```

## Widgets

Each widget is **display-only** in v1 — no click interactions.

### `glazewm` — workspace indicators

```toml
[widgets.workspaces]
type = "glazewm"
show_empty = false
```

Pills for each workspace from GlazeWM, focused one highlighted with `palette.accent`. Connects to `ws://127.0.0.1:6123` and reconnects with exponential backoff. **Hidden entirely** when GlazeWM isn't running.

### `clock` — formatted time

```toml
[widgets.clock]
type = "clock"
format = "%a %-d %b %-I:%M %p"
tick_seconds = 1
```

`format` is a [chrono strftime](https://docs.rs/chrono/latest/chrono/format/strftime/) string. The bar wakes once per `tick_seconds` to redraw.

### `sysinfo` — CPU / RAM / network / battery

```toml
[widgets.cpu]
type = "sysinfo"
metric = "cpu"           # "cpu" | "ram" | "network" | "battery"
icon = ""               # optional, drawn before the value
format = "{value:.0}%"
interval_seconds = 2
warn_above = 85.0        # optional, text uses warn_color above this
# warn_color = "#FF0000" # optional, defaults to palette.error
# charging_icon = "󰂄"   # battery only — used when on AC
```

Template vars exposed per metric:

| metric    | vars                                                                  |
|-----------|-----------------------------------------------------------------------|
| `cpu`     | `value` (%)                                                           |
| `ram`     | `value` (%), `used_gb`, `total_gb`, `free_gb`                         |
| `network` | `rx_bps`, `tx_bps`, `rx_kbps`, `tx_kbps`, `rx_mbps`, `tx_mbps`        |
| `battery` | `value` (%), `charging` (1.0 / 0.0)                                   |

Network has an extra `interface` field (`"*"` or omit → sum all interfaces; otherwise a specific name like `"Ethernet"`).

### `command` — arbitrary shell command

```toml
[widgets.weather]
type = "command"
command = "curl -s wttr.in/?format=3"
interval_seconds = 600
```

Runs the command in a background thread on the configured interval and shows the trimmed first line of stdout. Wrapped with `cmd /C` on Windows.

## Themes

Pick one in `config.toml`:

| name    | feel                                                           |
|---------|----------------------------------------------------------------|
| `Paper` | warm cream paper, ink black text, terracotta accent (default)  |
| `Stone` | cool grey, blue accent                                         |
| `Sage`  | muted greens                                                   |
| `Clay`  | warm earth tones                                               |
| `Ink`   | black background, off-white text (only dark theme)             |

Switch live from the terminal: `wbar set-theme Ink`.

## Customisation

### Palette overrides

Tweak individual colours of the selected theme:

```toml
[palette]
accent = "#3F5C32"
error  = "#B33525"
```

Each field is `"#RRGGBB"` or `"#RRGGBBAA"`. Any omitted field falls back to the theme's value.

### Token overrides

Spacing / radius / font-size tokens (used by widgets that respect them):

```toml
[tokens]
radius_sm = 6.0
font_body = 14.0
```

## Nerd Font icons

wbar scans `%LOCALAPPDATA%\Microsoft\Windows\Fonts` and `%WINDIR%\Fonts` at startup for a Nerd-Font-patched file (Symbols, JetBrainsMono, Iosevka, FiraCode, Hack — Mono variants preferred). The first hit is registered as a fallback in egui's monospace family, so any Nerd-Font glyph you embed in a widget's `icon` or `format` string renders correctly.

If a **SemiBold / Medium / Bold** variant of the same family is also present, it's used as the body font so all bar text renders heavier.

No fonts are bundled. Install one yourself — `JetBrainsMonoNerdFontMono-SemiBold.ttf` is a good default. See the [Nerd Fonts cheat sheet](https://www.nerdfonts.com/cheat-sheet) for icon codepoints.

## Control from the terminal

wbar listens on `127.0.0.1:17128`. The same `wbar.exe` binary works as a CLI client when invoked with a subcommand:

```powershell
wbar toggle              # show ↔ hide
wbar show                # ensure the bar is visible
wbar hide                # hide it (releases the AppBar reservation)
wbar quit                # exit the running bar
wbar set-theme Stone     # switch theme live
wbar --help              # show usage
```

### Bind to a hotkey

**AutoHotkey** (v2):

```ahk
#!b::Run "wbar.exe toggle"               ; Win+Alt+B → toggle
#!1::Run "wbar.exe set-theme Paper"
#!2::Run "wbar.exe set-theme Ink"
```

**GlazeWM** (in `config.yaml`):

```yaml
keybindings:
  - commands: ['shell-exec wbar.exe toggle']
    bindings: ['alt+b']
  - commands: ['shell-exec wbar.exe set-theme Ink']
    bindings: ['alt+shift+i']
```

Hiding releases the AppBar reservation so other maximised windows reflow up to full height; showing reclaims it.

## Building from source

Requires Rust stable + (on WSL) `mingw-w64`:

```bash
sudo apt install -y mingw-w64        # on WSL/Linux for cross-compiling
make build                            # cargo build --release --target x86_64-pc-windows-gnu
make install                          # copies wbar.exe to %USERPROFILE%\Documents\apps\
```

Targets and Make goals:

| goal        | what it does                                                       |
|-------------|--------------------------------------------------------------------|
| `make build`   | release build for `x86_64-pc-windows-gnu`                       |
| `make install` | build + kill any running `wbar.exe` + copy to install dir       |
| `make kill`    | taskkill any running `wbar.exe`                                 |
| `make clean`   | `cargo clean`                                                   |
| `make deploy`  | bump version, tag, push — triggers the GitHub release workflow  |

## Contributing

Bug reports and PRs welcome. Keep changes small and atomic; the codebase favours many short commits over rare large ones. Run before opening a PR:

```bash
cargo fmt --check
cargo clippy --target x86_64-pc-windows-gnu --all-targets -- -D warnings
cargo test --bins
```

The intended shape is small — single bar, single primary monitor, display-only widgets. Multi-monitor, click-to-act workspaces, plugin systems, and other features that grow the surface area are intentionally out of scope. New widgets and configuration knobs are welcome.

## License

[MIT](LICENSE).
