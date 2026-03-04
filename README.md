# logana

A fast, keyboard-driven terminal log viewer and analyzer with filtering, search, and annotations.

## Key Features

- Rich terminal UI with structured views, color themes, and keyboard-driven navigation
- Real-time filtering (include/exclude/date-range), search, and highlighting
- Persistent annotations, marks, and session restore across runs
- Multi-tab, vim keybindings, visual line mode, and clipboard integration

## Supported Log Formats

Auto-detected on open — no configuration required:

| Format | Examples |
|---|---|
| JSON | tracing-subscriber, bunyan, structured logging |
| Syslog | RFC 3164 (BSD), RFC 5424 |
| Journalctl | short-iso, short-precise, short-full |
| Common Log / Combined | Apache access, nginx access |
| Logfmt | Go slog, Heroku, Grafana Loki |
| Common log family | env_logger, tracing fmt, logback, Spring Boot, Python, loguru, structlog |
| Web error | nginx error, Apache 2.4 error |
| dmesg | Linux kernel ring buffer |
| Kubernetes CRI | Container Runtime Interface |

## Installation

### Pre-built binaries (recommended)

Download the latest release for your platform from the
[Releases page](https://github.com/pauloremoli/logana/releases), or use the
install scripts:

**Linux / macOS**
```sh
curl -fsSL https://github.com/pauloremoli/logana/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**
```powershell
irm https://github.com/pauloremoli/logana/releases/latest/download/install.ps1 | iex
```

### Homebrew (macOS / Linux)

```sh
brew tap pauloremoli/logana
brew install logana
```

### Cargo (from crates.io)

Requires the [Rust toolchain](https://rustup.rs).

```sh
cargo install logana
```

### Cargo (from source)

```sh
cargo install --git https://github.com/pauloremoli/logana
```

### AUR (Arch Linux)

```sh
paru -S logana
# or
yay -S logana
```

Manual install:
```sh
git clone https://aur.archlinux.org/logana.git
cd logana && makepkg -si
```

### Nix

```nix
# In a flake or overlay:
pkgs.callPackage (builtins.fetchGit {
  url = "https://github.com/pauloremoli/logana";
  ref = "main";
} + "/pkg/nix") {}
```

> **Note:** update the `hash` and `cargoHash` fields in `pkg/nix/default.nix`
> after each version bump (see comments in that file).
