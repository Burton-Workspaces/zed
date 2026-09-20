# Burton

Burton is a branded desktop editor built from [Zed](https://zed.dev). This folder is the **only** place Burton customizations live. Upstream Zed sources on `main` stay untouched so you can keep merging Zed updates without fighting icon, name, or copy conflicts.

You do not edit Zed files by hand. `burton/script/build` copies this repo into a throwaway worktree, applies the overlay, then compiles.

```
Zed sources (untouched)  +  burton/ overlay  →  branded Burton binary
```

This overlay is a GPL-3.0-or-later modification of Zed. Ship the license with binaries.

## What you get

Compared with a stock Zed build, Burton currently:

| Area | Burton |
|------|--------|
| Product name | **Burton** (dock, About, Welcome, menus, CLI, data dirs) |
| Command / binary | `burton` |
| URL scheme | `burton://` |
| App ID | `dev.burton.Burton` (plus `-Dev`, `-Preview`, `-Nightly`) |
| Dock / app icon | Mountain and lake mark in `burton/assets/` |
| In-app logo | `burton/assets/zed_logo.svg` (filename is kept on purpose) |
| Linux `.desktop` | Name, icon, and `burton` scheme |
| macOS bundle | `Burton.app`, DMG named `Burton-<arch>.dmg`, volume name `Burton` |
| Windows installer | `Burton.exe`, Inno setup named `Burton-<arch>.exe` |
| Auto-update | On, talking to Burton’s server (not zed.dev) |
| Extensions | Marketplace on the same origin; `burton-update-dev` proxies Zed |
| AI | Off |
| Telemetry | Off (diagnostics and metrics) |
| Account chrome | Sign-in, user menu, and user picture hidden |
| Leftover “Zed” copy | Product name swapped to Burton; other visible “Zed” chrome blanked or generic |

Identity values live in `branding.toml`. Default settings live in `settings-overlay.json`. Visible string swaps live in `replacements.toml`.

## Prerequisites

- Git, Python 3.11+, and a Rust toolchain that can build Zed
- Run **Linux** bundles on Linux, **macOS** bundles on a Mac, **Windows** bundles on Windows
- Linux: install Zed’s system libraries once with `script/linux` (needs sudo). That pulls in X11, Wayland, Vulkan, musl, and the rest of the compile toolchain. Without it, `bundle-linux` fails looking for `x11.pc` (`libx11-dev` on Debian/Ubuntu)
- macOS: Xcode command-line tools; `cargo-bundle` is installed by the bundle script if needed
- Windows: Visual Studio 2022, Inno Setup 6, and PowerShell

On Linux, before the first build:

```bash
script/linux
```

Icons are already generated. To recreate them:

```bash
burton/script/generate-assets
```

## How to build

Always use `burton/script/build`. It refuses to brand the checkout you are sitting in, so `main` stays a clean Zed tree.

### Day-to-day (run from source)

```bash
burton/script/build              # debug GUI → target/debug/burton
burton/script/build --release    # release GUI → target/release/burton
```

This is a cargo build with Burton branding applied. It does **not** produce an installer. Debug / `dev` channel builds do not auto-update.

### Production installers (one command per OS)

Bundle modes set the release channel to `stable` so the app will poll for updates.

```bash
burton/script/build bundle-linux      # on Linux
burton/script/build bundle-mac        # on macOS
burton/script/build bundle-windows    # on Windows
```

Optional channel:

```bash
burton/script/build --channel preview bundle-linux
```

Valid channels: `stable` (default for bundles), `preview`, `nightly`, `dev`.

Installers are copied to `target/`:

| OS | File | What it contains |
|----|------|------------------|
| Linux | `target/burton-linux-<arch>.tar.gz` | `burton.app` with `libexec/burton-editor` |
| macOS | `target/Burton-<arch>.dmg` | `Burton.app` on a volume named `Burton` |
| Windows | `target/Burton-<arch>.exe` | Inno installer that writes `Burton.exe` |

`<arch>` is `x86_64` or `aarch64`.

macOS and Windows bundlers read `target/` inside the worktree (not a shared `CARGO_TARGET_DIR`). Linux bundles share `target/` at the repo root. Either way, the files above are the ones to ship.

### Check leftover “Zed” strings

```bash
burton/script/build -- scan
```

This applies branding, then reports remaining user-visible “Zed” copy in scanned paths. It should print that none were found.

### Do not brand `main` in place

`burton/script/apply` will refuse to modify the primary git checkout. Do not pass `--force` on `main`. That flag exists only for emergency debugging.

## Auto-updates and extensions

Stable (and preview / nightly) Burton builds use **one origin** for auto-updates and the extension marketplace. Both are compiled from `branding.toml`:

```toml
update_server_url = "https://updates.burton.dev"
```

That value becomes `server_url`. On a custom host, the app does **not** remap to `cloud.zed.dev` / `api.zed.dev`, so this origin must implement both:

| App call | Path | Served by |
|----------|------|-----------|
| Auto-update | `GET /releases/{channel}/{version}/asset` | Local installer files |
| Installer download | `GET /files/{name}` | Local installer files |
| Extensions catalog / versions / updates | `GET /extensions`, `GET /extensions/updates`, `GET /extensions/{id}` | Reverse-proxy to Zed (`https://api.zed.dev` by default) |
| Extension install | `GET /extensions/{id}/download` | Reverse-proxy; **302s to Zed blob storage are followed server-side** so the client never sees those URLs |

Change the URL to your real server **before you ship**. At runtime you can override it without rebuilding:

```bash
ZED_SERVER_URL=http://127.0.0.1:4180 ./target/release/burton
```

Dev-channel builds never poll for app updates. `ZED_UPDATE_EXPLANATION` disables polling even on stable. Extensions still use the same origin.

### What the update JSON must look like

The query still says `asset=zed` — that is the protocol name, not the product name.

```
GET {update_server_url}/releases/{channel}/latest/asset?asset=zed&os={linux|macos|windows}&arch={x86_64|aarch64}
```

Respond with JSON:

```json
{
  "version": "1.22.1",
  "url": "https://updates.burton.dev/files/burton-linux-x86_64.tar.gz"
}
```

- `version` must be a newer semver than the running app, or nothing is installed
- `url` must point at the matching installer from the table above
- Linux tarball top-level folder must be `burton.app` (or `burton-preview.app` / `burton-nightly.app`) with `libexec/burton-editor`
- macOS DMG volume name must be `Burton`
- Windows installer must write `Burton.exe`

### Local server (`burton-update-dev`)

`burton-update-dev` is a Rust CLI (not part of the Zed workspace). After you have an installer in `target/`:

```bash
burton/script/serve-updates --dir target --version 1.22.1
```

Or:

```bash
cargo run --manifest-path burton/update-dev/Cargo.toml -- --dir target --version 1.22.1
```

That binds `http://127.0.0.1:4180` by default. Point a stable build at it with `ZED_SERVER_URL` as shown above.

Useful flags:

| Flag | Purpose |
|------|---------|
| `--dir` | Search directory for installers (repeatable; default `target/`) |
| `--version` | Version string in the release JSON (default: `crates/zed/Cargo.toml`) |
| `--host` / `--port` | Bind address (default `127.0.0.1:4180`) |
| `--listen` | Bind `host:port` in one flag (overrides `--host`/`--port`) |
| `--public-url` | Origin written into release `url` (use this behind Caddy) |
| `--extensions-upstream` | Marketplace to proxy (default `https://api.zed.dev`) |

If catalog listing 404s, try `--extensions-upstream https://cloud.zed.dev`.

### Caddy (TLS)

Caddy only terminates TLS. The CLI owns routing so extension download redirects stay on this origin:

```bash
burton-update-dev --listen 127.0.0.1:4180 --dir target --public-url https://updates.burton.dev
caddy run --config burton/update-dev/Caddyfile
```

`--public-url` is required when the bind address is loopback; otherwise release JSON would advertise `http://127.0.0.1:4180`. Without `--public-url`, the CLI uses `X-Forwarded-Proto` and `X-Forwarded-Host` from Caddy, then `Host`.

## Changing the brand

Edit files in `burton/` only, then rebuild.

| Want to change | Edit |
|----------------|------|
| App name, binary name, URL scheme, app ID | `branding.toml` |
| Update server host | `branding.toml` → `update_server_url` |
| Dock icon | Replace `assets/app-icon.png` (512×512) and `assets/app-icon@2x.png` (1024×1024) |
| Welcome / onboarding mark | Replace `assets/zed_logo.svg` (keep that filename) |
| Default settings (AI, telemetry, auto-update, title bar) | `settings-overlay.json` **and** the matching keys in `script/apply` |
| Visible “Zed” sentences | Add an exact `from` / `to` in `replacements.toml` |

Windows `.ico` files are generated from the PNG at apply time. You do not check them in.

Do **not** globally search-and-replace `"Zed"` in Zed sources. That breaks crate names, protocols, and tests. Prefer a unique `from` string in `replacements.toml`.

## Taking Zed updates

This repo tracks Zed and adds only `burton/`. Typical flow:

```bash
git fetch upstream
git merge upstream/main
burton/script/build -- scan
burton/script/build
```

If apply prints `missing in …`, Zed changed a string this overlay used to rewrite. Update `replacements.toml` or the choke-point list in `script/apply`. Add new user-visible “Zed” copy to `replacements.toml` rather than editing Zed sources.

## What’s in this folder

| Path | Role |
|------|------|
| `branding.toml` | Name, binary, scheme, app IDs, update server |
| `assets/` | Dock icons and in-app logo |
| `overlays/` | Full-file copies (Linux `.desktop` template) |
| `replacements.toml` | Allowlisted UI string edits |
| `settings-overlay.json` | Documented default-settings intent |
| `script/apply` | Copies overlays/assets and applies replacements |
| `script/build` | Throwaway worktree + apply + cargo or OS bundle |
| `script/generate-assets` | Regenerates the mountain-and-lake PNG icons |
| `script/serve-updates` | Launches `burton-update-dev` |
| `update-dev/` | Rust CLI: local updates + Zed extensions reverse-proxy |
| `update-dev/Caddyfile` | Example TLS terminator for `updates.burton.dev` |
| `script/scan-zed-strings` | Leftover-string report |

The root of this git repo remains a Zed tree. After a Burton build, `target/debug/zed` from an unbranded checkout is unchanged; the branded GUI is `target/debug/burton`.
