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
  posts events to a channel from its hidden Win32 window.
- `tray-icon` provides the tray icon and context menu and likewise posts menu events to a channel.
- Slint's winit backend owns the event loop and renders the config window when it is open.

Both `global-hotkey` and `tray-icon` expose a push handler stored in a process-wide `OnceCell`
that locks to the first value it sees; a single event arriving before the handler is installed
silently prevents it from ever being registered. To avoid that race, VOLE ignores the push handlers
and instead blocks on both event channels from two dedicated listener threads. Each forwards its
events to the main thread with `slint::invoke_from_event_loop`, which wakes the event loop even
while the window is hidden to the tray. Dispatch itself still runs on the loop thread, so it stays
single-threaded; the listeners only block on a channel `recv`, so idle CPU stays effectively zero.
A polling timer on the loop thread was tried first but stops firing once the window is hidden, which
left global hotkeys and the tray menu dead after the window was closed.

### Modules

Single binary crate with internal modules:

- `audio` — a cross-platform core plus swappable backends. `AudioBackend` is a trait
  (enumerate sessions, get/set volume and mute); `AudioEngine<B>` wraps a backend and owns the
  toggle snapshot state and the dispatch logic that resolves a target to session(s) and applies an
  operation, returning an `ApplyOutcome` describing what changed.
  - `audio::windows_backend` — `WindowsBackend`, the Core Audio (COM) implementation, compiled
    only on Windows.
  - `audio::fake` — `FakeBackend`, an in-memory backend seeded with representative sessions. It is
    always compiled (not `cfg`-gated) so both the Linux dev build and Windows integration tests can
    drive the engine without real audio hardware.
  - `platform_backend()` selects `WindowsBackend` on Windows and `FakeBackend` elsewhere behind the
    `PlatformBackend` type alias, so the rest of the app is platform-agnostic.
- `catalog` — a cross-platform core (curated common apps + `build`) that delegates OS-specific
  enumeration to a `platform` submodule: `catalog::platform_windows` (ToolHelp running processes +
  Start Menu `.lnk` scan) on Windows, `catalog::platform_fake` (empty impls) elsewhere.
- `hotkeys` — register hotkeys from config, map an incoming hotkey event to its bound actions.
  Registration and the `global-hotkey`/`tray-icon` event wiring are `#[cfg(windows)]`; on other
  platforms `register`/`unregister` are no-ops so the app still runs.
- `config` — load, validate, and save the JSON config; owns the typed schema.
- `tray` — tray icon and context menu (Open config, Enable/Disable, Quit).
- `ui` — the Slint config window, created lazily.
- `app` — orchestrator. Owns the in-memory config and an `AudioEngine<PlatformBackend>`, and
  exposes a single cross-platform `fire(index)` dispatch path shared by real hotkeys and the UI's
  Test-fire button.
- `autostart` — toggles a logon-triggered scheduled task registered with the highest privileges.

### Audio targeting

On each hotkey press the core re-enumerates audio sessions rather than caching them. Enumeration
is sub-millisecond and this keeps behaviour robust as apps open and close.

- **Named process** target (e.g. `Discord.exe`): match every session whose process image matches
  the name, and apply the op to each. A name matching several sessions naturally covers apps that
  spawn multiple audio sessions.
- **Foreground window** target: `GetForegroundWindow` -> `GetWindowThreadProcessId` -> match the
  session with that PID. If the focused app has no audio session, the action is a no-op. This
  absence is handled explicitly rather than assumed away.

### Dispatch outcome and feedback

Applying a hotkey returns an `ApplyOutcome { affected, restored }` rather than nothing. This makes
the common "I pressed the key and nothing happened" case observable instead of silent: a hotkey can
legitimately match zero sessions (the target app is not currently producing audio, or a
`foreground` action fires while VOLE itself is focused), which previously looked identical to a
broken binding. When `affected` is empty the UI reports "no matching audio session", and the config
window's per-app Test-fire button runs the exact same dispatch path so a binding can be diagnosed
without touching real hotkeys.

Core Audio call chain: `IMMDeviceEnumerator` -> default render device -> `IAudioSessionManager2`
-> `IAudioSessionEnumerator` -> per-session `IAudioSessionControl2` (for PID/name) and
`ISimpleAudioVolume` (for volume and mute).

### Hotkeys, actions, and operations

A hotkey owns a **list of actions** plus a **toggle** flag. Each action pairs one **target** with
one **operation**. The action list is what lets a single hotkey do several unrelated things at once
— for example, relative-adjust one app down, set another to 50%, and mute a third, all on the same
keypress.

Targets:

- `process` — a named executable.
- `foreground` — the currently focused window.

Operations:

- `set` — set the target to an absolute level (0.0–1.0).
- `adjust` — nudge the target by a relative delta (e.g. -0.10).
- `mute` — set the target's mute state (`{ "muted": true | false }`).

### Toggle mode

Any hotkey can be marked `toggle`. On the first press VOLE snapshots the level and mute state of
every session the hotkey touches, then applies its actions. On the next press it restores that
snapshot instead of re-applying. This generalises the old duck/mute-toggle behaviour to every
operation: a `set`-to-low hotkey becomes a duck, a `mute` hotkey becomes a mute-toggle, and so on.
The snapshot is held in memory, keyed per hotkey by resolved session PID, and is not persisted — a
fresh launch starts with a clean slate.

### Config schema

Stored at `%APPDATA%\VOLE\config.json`. Schema-driven: the raw file is normalized, validated once
(defaults applied, duplicate keybinds and conflicts detected), then mapped declaratively to runtime
handlers with no further branching. Older `version: 1` files are migrated in place on load
(`toggle_mute`/`toggle_duck` operations become `mute`/`set` with the hotkey's `toggle` flag set).

```json
{
  "version": 2,
  "hotkeys": [
    {
      "id": "gaming-focus",
      "name": "Gaming focus",
      "shortcut": "Ctrl+Alt+G",
      "enabled": true,
      "toggle": false,
      "actions": [
        {
          "target": { "type": "foreground" },
          "operation": { "type": "set", "level": 0.3 }
        },
        {
          "target": { "type": "process", "executable": "Spotify.exe" },
          "operation": { "type": "adjust", "delta": -0.2 }
        },
        {
          "target": { "type": "process", "executable": "chrome.exe" },
          "operation": { "type": "mute", "muted": true }
        }
      ]
    },
    {
      "id": "hear-discord",
      "name": "Hear Discord",
      "shortcut": "Ctrl+Alt+D",
      "enabled": true,
      "toggle": true,
      "actions": [
        {
          "target": { "type": "foreground" },
          "operation": { "type": "set", "level": 0.2 }
        }
      ]
    }
  ]
}
```

### Elevation

VOLE embeds a Windows application manifest (`build.rs`, via `embed-manifest`) requesting
`requireAdministrator`, so it always runs at high integrity. This is required for hotkeys: Windows'
User Interface Privilege Isolation suppresses `WM_HOTKEY` delivery to a lower-integrity process
while an elevated window holds the foreground, which is exactly the case for anti-cheat protected
games (e.g. Rainbow Six Siege under BattlEye). Running elevated keeps `RegisterHotKey` — still the
anti-cheat-safe mechanism — working even when such a game is focused.

Because the app requires elevation, the installer installs to Program Files (`{autopf}\VOLE`) under
an admin-elevated setup, rather than a user-writable directory. An elevated executable living in a
user-writable location would be a privilege-escalation risk.

### Autostart

Autostart runs VOLE through a logon-triggered scheduled task registered with the highest privileges
(`schtasks /Create /SC ONLOGON /RL HIGHEST`), not an `HKCU\...\Run` value. A Run entry would raise a
UAC prompt at every logon now that the app requires administrator rights; the scheduled task
launches elevated silently instead. The `autostart` module owns the `schtasks` calls, and the app
exposes `--enable-autostart` / `--disable-autostart` CLI flags that the elevated installer invokes
to add the task (when the startup option is checked) or the uninstaller invokes to remove it. On
launch the app mirrors the task's presence back into `config.launch_on_startup` so the stored flag
stays honest.

---

## Dependencies (Rust crates)

- `windows` (windows-rs) — Core Audio (COM) and foreground-window / PID lookup. Declared as a
  target-only dependency (`[target.'cfg(windows)'.dependencies]`).
- `global-hotkey` — global hotkeys; `RegisterHotKey`-based on Windows, anti-cheat-safe. Target-only.
- `tray-icon` — tray icon and context menu. Target-only.
- `png` — decodes the bundled `assets/vole.png` mascot into RGBA for the tray icon. Target-only.
- `slint` (winit backend) — the animated config window. A normal dependency so the GUI builds and
  runs on Linux too.
- `serde` + `serde_json` — config load/save.
- `embed-manifest` (build dependency) — embeds the `requireAdministrator` Windows manifest. Works
  when cross-compiling from Linux, so no external MinGW/LLVM tooling is needed.

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

The stable Inno `AppId` makes subsequent installers upgrades rather than side-by-side installs.
An upgrade closes the running process and replaces app files in place. Configuration lives outside
the install directory under `%APPDATA%`, so upgrades preserve it.

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

The real audio and hotkey layers are Windows-only (Core Audio, `RegisterHotKey`), but the crate is
structured around an `AudioBackend` trait so the whole app — including the Slint config window —
builds and runs on Linux against the in-memory `FakeBackend`. Global-hotkey and tray wiring are
`cfg`-gated to no-ops off Windows.

- **On Linux:** `cargo run` launches the full GUI against the fake backend, so the config window,
  app catalog rows, live level/mute readouts, and Test-fire button can all be exercised end to end.
  The Astro site develops fully as well.
- **Testing:** unit tests cover the `AudioEngine` dispatch (set/adjust/mute/toggle-restore/
  foreground/process-match) and `tests/dispatch.rs` drives a GUI-style config JSON through the
  engine, all on Linux against the fake backend.
- **Compile-checking Windows:** `cargo clippy --target x86_64-pc-windows-gnu` (or `cargo-xwin`)
  cross-checks the Windows-only code from Linux. Note `cargo fmt` skips `#[cfg(windows)]` modules,
  so run `rustfmt --edition 2024` directly on `audio/windows_backend.rs` and
  `catalog/platform_windows.rs`.
- **Functional Windows testing:** still done on a real Windows PC via a draft-release build.

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
