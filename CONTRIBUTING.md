# Contributing to Porchly

Thanks for helping. Porchly's bar is that **someone's grandpa can download it, double-click it
and have a working API without help**, and that nothing it does by default puts their data at
risk. Every change is judged against that.

The full rules (safety, never losing data, never crashing, layering, plain language) are in
[CLAUDE.md](CLAUDE.md). They're written for AI coding agents and apply equally to people.

## Building from source
You need [Rust](https://rustup.rs) (stable) and a C compiler (SQLite is built in). On Linux,
the window also needs the usual X11/Wayland libraries.

```sh
git clone https://github.com/YasserManss/porchly
cd porchly
cargo run --release -p porchly -- --sample        # the app, with a sample table
cargo build --release -p porchly-cli              # the command-line tool, see docs/cli.md
```

Without `--project`, the app keeps its data in `Porchly data/` next to the program (here,
`target/release/`), or in `~/Porchly` if that's where your data already is.

## Before every commit
```sh
cargo xtask ci
```
It runs `cargo fmt --check`, `clippy -D warnings`, all tests, and Porchly's own checks: every
file under `assets/` is listed in [ATTRIBUTION.md](ATTRIBUTION.md), and crates only depend on
lower layers. The same check runs on Linux, Windows and macOS for every push.

Some tests need real helpers or the internet and are skipped unless you point them at one:

```sh
PORCHLY_TEST_BUN=/path/to/bun cargo test -p porchly-engine --test functions
PORCHLY_TEST_CLOUDFLARED=/path/to/cloudflared cargo test -p porchly-engine --test porch_light
```

## Looking at the app without a window
```sh
cargo run -p porchly-ui-egui --example snapshot -- out.png --sample orders --table orders --share
```
renders the app to a PNG (`--dark`, `--dialog new_table|settings|porch_light|…`, `--scale 2`;
the file's header lists every option). The screenshots in `assets/screenshots/` are made this
way. Click-through tests live in `crates/ui-egui/tests/ui.rs`. For UI changes, look at the
result and ask: could someone who has never heard of an API finish this without help?

A running app can be driven by scripts and agents over its control channel
(`porchly --control 7979`, see [docs/control-protocol.md](docs/control-protocol.md)).

## How the code is organised
One Rust workspace; a crate may only use crates in its own or a lower layer
(`cargo xtask layers` enforces it).

| Layer | Crates | What |
|---|---|---|
| 0 | `core` | errors, validated names, field kinds, limits |
| 1 | `schema`, `store`, `fetch` | values and tables; all SQL (SQLite); checked downloads |
| 2 | `api` | the HTTP API (axum) |
| 3 | `functions`, `tunnel` | functions run with Bun; the porch light (cloudflared) |
| 4 | `engine` | every action is a command: `Engine::execute(id, params)` |
| 5 | `ui-egui` | the desktop interface (egui) |
| 6 | `apps/porchly`, `apps/porchly-cli` | the programs |

Everything a person can do is a command in `crates/engine/src/cmd/`, so the app, the command
line and scripts all do the same thing. `porchly-cli commands` lists them.

## Releasing
Push a version tag (`git tag v0.2.0 && git push --tags`). `.github/workflows/release.yml`
builds the Windows, macOS and Linux downloads and publishes a GitHub release.

## Reporting problems
Bugs and ideas: [GitHub issues](https://github.com/YasserManss/porchly/issues). Security
problems: privately, see [SECURITY.md](SECURITY.md).

By contributing you agree that your work is licensed under MIT or Apache-2.0, like the rest of
Porchly.
