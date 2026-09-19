# Burton branding overlay

This directory is the only place Burton customizations live. Upstream Zed files on `main` stay untouched so `git merge upstream/main` does not conflict with icons, names, or copy.

```
upstream/main  ->  origin/main (clean Zed tree + this folder)
                      |
                      +--> burton/script/build  (worktree + apply + cargo)
```

## What it changes

- Product name: **Burton** (dock, About, Welcome, CLI, data dirs)
- URL scheme: `burton://`
- Binary / CLI: `burton`
- App ID: `dev.burton.Burton` (and channel suffixes)
- Icons: `burton/assets/app-icon.png` and `zed_logo.svg` copied over Zed’s filenames at apply time
- Leftover “Zed Industries” chrome is blanked
- Defaults: auto-update off, telemetry off, AI off, title-bar user/sign-in chrome hidden

## Build

```bash
burton/script/generate-assets    # once, if icons are missing
burton/script/build              # cargo build -p zed in a worktree
burton/script/build --release
burton/script/build -- bundle-linux
burton/script/build -- scan      # leftover "Zed" report after apply
```

`apply` will refuse to modify the primary checkout. Do not pass `--force` on `main`.

The GUI binary is `target/debug/burton` (or `target/release/burton`). Official Zed’s `target/debug/zed` is unchanged because branding never hits this tree.

## Replace the placeholder logo

Drop your art here, keeping these names:

- `burton/assets/app-icon.png` — 512×512 dock icon
- `burton/assets/app-icon@2x.png` — 1024×1024
- `burton/assets/zed_logo.svg` — in-app Welcome / onboarding mark (filename is kept on purpose)

Windows `.ico` files are generated from the PNG at apply time.

## Upstream updates

```bash
git fetch upstream
git merge upstream/main
burton/script/build -- scan
burton/script/build
```

If apply prints `missing in …`, upstream changed a branded string. Update `burton/replacements.toml` or the choke-point list in `burton/script/apply`. Add new user-visible “Zed” copy to `replacements.toml` rather than editing Zed sources.

## Layout

| Path | Role |
|------|------|
| `branding.toml` | Name, bin, scheme, app IDs |
| `assets/` | Icons and in-app logo |
| `overlays/` | Full-file copies (currently `.desktop`) |
| `replacements.toml` | Allowlisted UI string edits |
| `settings-overlay.json` | Documented default-settings intent |
| `script/apply` | Copies overlays/assets and applies replacements |
| `script/build` | Disposable worktree + apply + cargo |
| `script/scan-zed-strings` | Leftover-string report |

This overlay is a GPL modification of Zed; ship it with binaries.
