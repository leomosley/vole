# VOLE

Volume Output Level Executioner is a lightweight Windows tray application for controlling
per-application volume with global hotkeys.

See [ARCHITECTURE.md](./ARCHITECTURE.md) for the planned system design.

## Installing

Download the latest `VOLE-Setup-*.exe` from
[GitHub Releases](https://github.com/leomosley/vole/releases). Contributor test builds are
published as draft releases from `dev`.

## Development

```sh
bun install
bun run typecheck
cargo test --workspace
```
