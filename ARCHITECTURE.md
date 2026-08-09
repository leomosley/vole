# VOLE — Architecture

**VOLE** — Volume Output Level Executioner. A lightweight Windows tray application that binds
global hotkeys to per-application volume changes via the Windows volume mixer. Built for gamers
who want to duck their game or Discord on the fly without alt-tabbing.

---

## Goals

- Global hotkeys that change per-application volume through the Windows volume mixer.
- One hotkey can drive multiple, independent actions across different applications at once.
- Always running, with a near-zero resident footprint (single-digit MB RAM, ~0% idle CPU).
- A fun, animated configuration UI — but one that costs nothing while gaming.
- Simple distribution: a downloadable installer, auto-built by CI.

## Non-goals

- Cross-platform support. VOLE is Windows-only by nature of the Core Audio APIs it uses.
- A system-wide EQ, audio routing, or virtual device features. Volume control only.
- A cloud service or account system. Everything is local and client-side.

## Constraints

- Must not trip kernel-level anti-cheat (Vanguard, EAC, BattlEye). This rules out low-level
  keyboard hooks and dictates the use of the standard `RegisterHotKey` mechanism.
- The resident process must stay tiny at all times, including while a game is running.

---

## High-level architecture

A single native Rust binary that lives in the system tray.

- A resident core runs a single event loop that services global hotkeys and the tray menu.
- On a hotkey press, the core resolves the bound actions and pokes Core Audio to change volume.
- The Slint configuration window is created lazily — only when the user opens it — and destroyed
  on close. Its memory and GPU cost exist only while it is open, so gaming pays nothing for it.
- Configuration is persisted as JSON in `%APPDATA%`. UI edits mutate the in-memory model and
  rewrite the file; because the UI and core share one process, hotkeys re-register immediately
  with no IPC.

```
+-------------------------------------------------------------+
|  vole.exe (single process)                                  |
|                                                             |
|  winit event loop (main thread, ~0% idle)                   |
|    - global-hotkey events  --> resolve actions --> audio    |
|    - tray-icon menu events --> open config / quit           |
|                                                             |
|  audio (windows-rs Core Audio)  <-- called on hotkey        |
|  config (serde JSON in %APPDATA%)                           |
|  ui (Slint window, created lazily on "Open config")         |
+-------------------------------------------------------------+
```

---

## Monorepo layout

Mixed TypeScript + Rust. A Bun/Turbo root drives the website; a Cargo workspace drives the app.

```
vole/
├─ apps/
│  ├─ web/            # Astro + TailwindCSS, no React. Landing / download page (Vercel).
│  └─ desktop/        # Single Rust crate: the VOLE tray app.
├─ packages/          # Empty for now. Reserved for future crate splits.
├─ scripts/
├─ .github/
│  ├─ CODEOWNERS
│  ├─ dependabot.yml
│  └─ workflows/
│     ├─ check.yml    # TS typecheck/lint + cargo fmt/clippy/test.
│     └─ release.yml  # Windows build -> Inno Setup -> GitHub Release (custom).
├─ .vscode/
├─ package.json       # Bun/Turbo workspace root.
├─ turbo.json
├─ tsconfig.json
├─ bunfig.toml
├─ Cargo.toml         # Cargo workspace; Rust crates are added explicitly.
├─ rust-toolchain.toml
├─ .prettierrc.astro
├─ .prettierignore
├─ .gitignore
└─ README.md  LICENSE  CONTRIBUTING.md
```

---

## Desktop app design (`apps/desktop`)

### Event loop model

A single Slint event loop on the main thread services everything. This avoids juggling multiple
OS message loops or threads.

- `global-hotkey` registers hotkeys (it uses `RegisterHotKey` under the hood on Windows) and
  forwards events from its hidden Win32 window to the Slint loop.
- `tray-icon` provides the tray icon and context menu and uses the same thread's Win32 message
  queue.
- Slint's winit backend owns the event loop and renders the config window when it is open.

When idle, the loop is blocked waiting for OS events, so CPU use is effectively zero.

### Modules

Single binary crate with internal modules:

- `audio` — enumerate audio sessions, resolve a target to session(s), get/set volume, toggle mute.
- `hotkeys` — register hotkeys from config, map an incoming hotkey event to its bound actions.
- `config` — load, validate, and save the JSON config; owns the typed schema.
- `tray` — tray icon and context menu (Open config, Enable/Disable, Quit).
- `ui` — the Slint config window, created lazily.
- `app` — orchestrator. Owns the in-memory config and applies resolved actions to `audio`.
- `autostart` — toggles the `HKCU\...\Run` registry key.

### Audio targeting

On each hotkey press the core re-enumerates audio sessions rather than caching them. Enumeration
is sub-millisecond and this keeps behaviour robust as apps open and close.

- **Named process** target (e.g. `Discord.exe`): match every session whose process image matches
  the name, and apply the op to each. A name matching several sessions naturally covers apps that
  spawn multiple audio sessions.
- **Foreground window** target: `GetForegroundWindow` -> `GetWindowThreadProcessId` -> match the
  session with that PID. If the focused app has no audio session, the action is a no-op. This
  absence is handled explicitly rather than assumed away.

Core Audio call chain: `IMMDeviceEnumerator` -> default render device -> `IAudioSessionManager2`
-> `IAudioSessionEnumerator` -> per-session `IAudioSessionControl2` (for PID/name) and
`ISimpleAudioVolume` (for volume and mute).

### Hotkeys, actions, and operations

A hotkey owns a **list of actions**. Each action pairs one **target** with one **operation**.
This is what lets a single hotkey do several unrelated things at once — for example, relative-adjust
one app down, set another to 50%, and mute a third, all on the same keypress.

Targets:

- `process` — a named executable.
- `foreground` — the currently focused window.

Operations:

- `set` — set the target to an absolute level (0.0–1.0).
- `adjust` — nudge the target by a relative delta (e.g. -0.10).
- `mute_toggle` — mute or unmute the target.
- `toggle_duck` — press once to drop the target to a low level, press again to restore the level
  it had before ducking. This is the primary "let me hear Discord" operation and is the one op
  that carries per-hotkey runtime state (the remembered pre-duck level).

### Config schema

Stored at `%APPDATA%\VOLE\config.json`. Schema-driven: the raw file is normalized, validated once
(defaults applied, duplicate keybinds and conflicts detected), then mapped declaratively to runtime
handlers with no further branching.

```json
{
  "version": 1,
  "hotkeys": [
    {
      "id": "gaming-focus",
      "keys": "Ctrl+Alt+G",
      "actions": [
        {
          "target": { "type": "foreground" },
          "op": { "type": "set", "level": 0.3 }
        },
        {
          "target": { "type": "process", "name": "Spotify.exe" },
          "op": { "type": "adjust", "delta": -0.2 }
        },
        {
          "target": { "type": "process", "name": "chrome.exe" },
          "op": { "type": "mute_toggle" }
        }
      ]
    },
    {
      "id": "hear-discord",
      "keys": "Ctrl+Alt+D",
      "actions": [
        {
          "target": { "type": "foreground" },
          "op": { "type": "toggle_duck", "low": 0.2 }
        }
      ]
    }
  ]
}
```

### Duck state

`toggle_duck` needs to remember the pre-duck level so it can restore it on the second press. This
state is held in memory, keyed per hotkey (and per resolved session), and is not persisted — a
fresh launch starts un-ducked.

### Autostart

An in-app "Launch on startup" toggle writes or removes the
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value. Because the app can register its own
autostart, it is fully functional even when run as a portable binary, and the installer's startup
option simply sets the same key.

---

## Dependencies (Rust crates)

- `windows` (windows-rs) — Core Audio (COM) and foreground-window / PID lookup. Declared as a
  target-only dependency (`[target.'cfg(windows)'.dependencies]`).
- `global-hotkey` — global hotkeys; `RegisterHotKey`-based on Windows, anti-cheat-safe.
- `tray-icon` — tray icon and context menu.
- `slint` (winit backend) — the animated config window.
- `serde` + `serde_json` — config load/save.

No async runtime. The app is event-loop driven, which keeps the binary and RAM small.

---

## Website (`apps/web`)

A small static Astro + TailwindCSS site with no React. Its only jobs are to describe VOLE and to
provide a download button that links to the latest published GitHub Release asset. Because it does
not host the binary itself, its bandwidth cost is trivial and it fits comfortably on Vercel's free
tier via Vercel's native Git deploy integration.

---

## Build, CI, and distribution

### Where binaries live

- The **website** is hosted on Vercel (free tier), serving only small static assets.
- The **installer** is hosted as a **GitHub Release asset**. GitHub absorbs the binary bandwidth,
  which keeps Vercel usage negligible.

### Installer

Inno Setup produces a small `.exe` installer: it installs the binary, adds a Start Menu shortcut
and an Add/Remove Programs entry, offers a "Launch on startup" checkbox that sets the `Run` key,
and ships a clean uninstaller.

### Public releases (`release.yml`)

Triggered on a version tag on `main`. On a `windows-latest` runner: build the release binary,
run Inno Setup, and attach the installer to a **published** GitHub Release. This is the version
end users download from the website.

### Contributor-only test builds

Test builds are distributed as **draft releases**, which on a public repo are visible and
downloadable only by collaborators with write access.

- Push to a `dev` branch -> CI builds the Windows installer -> upsert a draft release (e.g. tag
  `dev-latest`) with the installer attached.
- Codeowners download it from the Releases page and test on a real Windows machine.
- The public never sees draft releases. `main` is only touched when publishing a real version.

Note: the "pre-release" flag does **not** restrict access on a public repo — only drafts and
private repos do. Drafts are the mechanism used here.

### Release optimization

`opt-level = "z"`, `lto = true`, `panic = "abort"`, and stripped symbols keep the binary small.

---

## Developing on Linux

The desktop crate is inherently Windows-only (Core Audio, `RegisterHotKey`), so it does not build
or run natively on Linux, and no platform-abstraction layer is maintained to change that.

- **On Linux:** write code, edit and preview the Slint UI (Slint renders `.slint` files natively
  on Linux), and develop the Astro site fully.
- **Compile-checking:** happens in CI. Optionally, `cargo-xwin` can cross-compile to the Windows
  target from Linux for a quick local build sanity check, with no extra code required.
- **Functional testing:** done on a real Windows PC by downloading a draft-release build.

---

## Footprint summary

- Resident: single-digit MB RAM, ~0% CPU idle (event loop blocked on OS events).
- The Slint config window's cost is incurred only while it is open, never during gaming.
- No async runtime, no bundled browser engine, no localhost server.

---

## Open questions / future

- Code signing: unsigned installers trigger Windows SmartScreen "unknown publisher." Ship unsigned
  initially; a signing certificate can be added later to remove the warning.
- Whether to also ship a portable single-exe alongside the installer (the app self-manages
  autostart, so this is feasible later without architectural change).
- Multi-output-device handling if a user routes apps to different devices.
