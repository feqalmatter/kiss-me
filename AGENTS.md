# kiss-me

A small Cyberpunk 2077 mod enabler. The `cursor/rewrite-tui-mod-manager-*` branch is a Rust rewrite
exposing both a CLI and an interactive TUI (`crossterm` + `ratatui`); `main` is still the original
`kiss-me.sh` shell script.

## Cursor Cloud specific instructions

Scope of this section: durable, non-obvious context for the Rust TUI branch. Standard commands live in
`Cargo.toml` / cargo conventions.

### Toolchain

- Requires Rust **>= 1.85** (a transitive dep, `toml_parser`, needs edition 2024). The base VM image
  pins the default toolchain to **1.83.0** (`RUST_VERSION=1.83.0`), which fails to even parse the
  dependency manifests. The update script runs `rustup default stable` to switch to a modern stable;
  if you start a shell before the update script has run, run `rustup default stable` yourself.

### Build / lint / test / run

- Build: `cargo build`
- Lint: `cargo clippy --all-targets` (clean)
- Test: `cargo test` (currently no tests defined)
- Run CLI: `cargo run -- <subcommand>` (e.g. `config-path`, `enable <mod>`, `disable <mod>`, `check-library`)
- Run TUI: `cargo run -- tui` (or just `cargo run`, since `tui` is the default subcommand)

### Non-obvious caveats

- **TUI needs a real terminal.** It enters raw mode + the alternate screen, so it only renders in an
  interactive terminal. For manual testing, run it inside a desktop terminal emulator, not via piped
  stdout. Keys: `Tab` switches panes, arrows/`j`/`k` move, `Space` toggles the selected mod, `Enter`
  runs the focused command, `q`/`Esc` quits.
- **`assemble` / `generate-overwrite` cannot run in this VM.** They require the `OUTPUT` parent to sit
  on a reflink-friendly filesystem (`btrfs`/`zfs`/`xfs`/`bcachefs`) because they copy the game with
  `cp --reflink=auto`. The cloud VM's working dirs are on `overlay`, so these commands fail the
  filesystem precheck by design. Mod enable/disable/list/check-library work fine.
- **`cargo fmt --check` reports diffs** against the committed `src/tui.rs` (it is not rustfmt-clean).
  This is pre-existing; do not reformat unrelated code just to satisfy the check.
- **Config resolution order:** `--config` flag, then `$KISS_ME_CONFIG`, then `./kiss-me.toml`, then
  `~/.config/kiss-me/config.toml`. Paths can also be overridden per-run via env vars
  (`GAME_SOURCE`, `LIBRARY`, `DOWNLOADS`, `OUTPUT`, `MANIFEST`, `NEXUS_GAME_DOMAIN`). `~` is expanded.
- **Nexus downloads** need an API key via `nexus_api_key` in config or `NEXUS_API_KEY` /
  `NEXUSMODS_API_KEY` env vars, and reach out to the real Nexus Mods API.
