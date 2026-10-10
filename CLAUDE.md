# Porchly — instructions for agents

Porchly is an open-source, Rust-native "backend in a box": one double-clickable binary that gives anyone a database, a live API and a public link. You create a table that looks like a spreadsheet, and it is served immediately as a REST API, with an optional tunnel to a public HTTPS URL. Plugins (sandboxed WASM) add email, notifications, external databases and external APIs. It runs natively on macOS, Windows and Linux. The bar for ease of use: **someone's grandpa can download it, double-click it, and have a working API without opening a terminal.**

## Start every session here
1. Read `plan/STATUS.md` (current milestone, next task), then that task in `plan/execution-plan.md` and the relevant `plan/architecture.md` section.
2. Run `cargo xtask ci` before changing anything, so you know the tree is green.
3. Work autonomously. Don't stop to ask unless `plan/execution-plan.md` §7 lists the decision as the user's; record those in `plan/STATUS.md` and carry on with something else.

`plan/` is gitignored (local only).

## Safe by default — absolute rule
**People put their data on the internet with Porchly without understanding what that means. Every default must protect them.** This is the most important rule in this repository; breaking it is a serious failure.
- **Nothing is public until the user explicitly makes it public.** The API binds to `127.0.0.1` by default. The tunnel is off until the user switches it on, and switching it on shows a plain-language warning first.
- **Every table requires an API key from the start.** Tables reached through the tunnel are read-only unless the user ticks "allow edits". Rate limiting is always on.
- **The tunnel serves only the Porchly API**, never arbitrary files or proxied content. That keeps abuse low and the relay defensible.
- **Secrets live in the OS keychain** (`keyring` crate), never in the database, config files, logs, error messages, crash reports or plugin storage. Logs redact `Authorization` headers, API keys and anything marked `secret`.
- **All input is hostile:** HTTP requests, query strings, JSON bodies, imported CSV/files, plugin outputs, tunnel traffic and control/MCP params. Validate at the boundary and cap sizes (`porchly_core::limits`: request bodies, page sizes, filter counts, import rows).
- **SQL:** values are always bound parameters. Identifiers can't be bound, so they enter SQL only as `Ident::quoted()` (validated in `porchly-core`). All SQL lives in `porchly-store`; `cargo xtask layers` fails if another crate depends on a database driver.
- **Never needs administrator rights.** Default port 11911 (never below 1024), no system folders, no background services.
- **Portable, no installer** (owner decision): Porchly ships as one file per platform that runs from anywhere (a downloads folder, a USB stick). Nothing is installed or registered; deleting it uninstalls it. Never add an installer, registry entries or system-wide files.
- **No telemetry** unless the user opts in. No network calls at startup other than the opt-in update check.
- **Every security fix lands with a regression test** in `crates/api/tests/security.rs` or the crate it touches.

## Never lose data
People trust Porchly with their records; losing them is worse than a crash.
- SQLite in WAL mode, one `.db` file per project. Every write goes through a transaction.
- **Every schema change is a migration** (`crates/store/src/migrate.rs`) and takes an automatic snapshot first. Destructive changes (drop table, drop column, change type) are undoable for the session and need confirmation in the UI.
- Automatic daily backups (keep 7) next to the project file. Restoring is one click.
- Never write a project file in a format an older release can't detect: bump `store::FORMAT_VERSION` and refuse politely when opening a newer file.

## Never crash
A crash takes the user's API offline. **This outranks feature work.**
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- **Errors are `Result<T, E>`** through the crate's error type (`thiserror`) and `?`. Errors shown to the user say what happened and what to do next, in plain language.
- **Input-derived numbers are hostile:** `get()` instead of indexing, checked/saturating arithmetic, cap allocations sized by input.
- **Bound recursion and loops:** filter nesting, pipeline depth, automations that trigger other automations (cap the chain and detect cycles).
- **Last-resort guard:** `Engine::execute` catches escaped panics and rolls back the transaction; each HTTP request and each plugin call is isolated, so one bad request never takes the server down. It's a safety net, not a licence.
- **Prove it:** every crash fix lands with a regression test (`tests::every_command_survives_empty_params` fuzzes every command).
- Enforced by workspace lints (`[workspace.lints]` in `Cargo.toml`; every crate has `[lints] workspace = true`): `unwrap_used`, `expect_used`, `panic`, `unimplemented`, `todo`, `unreachable` and `indexing_slicing` are denied. Tests may unwrap (`clippy.toml`); integration test files start with an `#![allow(...)]` for the same lints.

## Plugins
- **Plugins are WASM components** run in `wasmtime`. The contract is the WIT package in `wit/` (`porchly:plugin@<version>`).
- **A published WIT version is frozen.** Changes are additive: add `@1.1` next to `@1.0` and keep the host supporting both. `cargo xtask wit-check` fails on a breaking change to a published version.
- **Plugins and scripts are trusted, not controlled** (owner decision). Installing a plugin or adding a script means accepting its risk. Porchly does not restrict or monitor them: no permission lists, no approved-sites list, no consent prompts, no per-call limits, no logging of what they do. They get the real `fetch()` to any address, every table, and secrets by name. Don't add sandbox policy back without the owner asking.
- **Say it once, plainly:** the install screen tells people that plugins can do anything with their data and the internet, aren't checked by Porchly, and should only come from people they trust. Informing is fine; gating is not.
- **Still never crash:** plugins and scripts run off the request path on their own threads, so one that fails or hangs can't crash or freeze Porchly. Its error is shown and logged as an error, nothing more.
- **User scripts are JavaScript/TypeScript** (StarlingMonkey via componentize-js), run as components like plugins, under the same rules.
- **Plugins never draw UI.** They declare a config schema in `plugin.toml`; `ui-egui` renders the form.
- **Build features as plugins when you can.** CSV import, webhooks and tunnel providers live in `plugins/` and use the public API. If the public API can't express it, the API has a gap: fix the API rather than adding a private back door.
- Native code is reserved for things WASM can't do well yet: database drivers in `crates/connectors` (exposed to plugins through `host.db-connect`) and the tunnel client.

## Non-negotiables
- **Everything is a command.** User-visible behaviour = a command in `crates/engine/src/cmd/*` (id, label, menu path, shortcut, params doc, `enabled`, `run`) + tests. Ids are namespaced by area: `table.create`, `field.add`, `row.insert`, `api.key.create`, `tunnel.start`, `plugin.install`, `automation.create`. UI-only commands live in `crates/ui-egui`. The GUI, CLI, control channel and MCP server all reach the same commands.
- **Programmatic calls never open dialogs:** a command with empty params runs with defaults or returns an error; only menu-style invocation opens a dialog (`UiRequest::Dialog`).
- **Layering** is enforced by `cargo xtask layers`: `core` (L0) → `schema`/`store` (L1) → `pipeline`/`api`/`automate` (L2) → `plugin-host`/`connectors`/`tunnel`/`functions` (L3) → `engine` (L4) → `ui-egui`/`mcp`/`control` (L5) → `apps/*` (L6). Nothing below L5 depends on egui/eframe/winit/rfd. **The UI crate is swappable**, and the engine runs headless (`porchly-cli serve`).
- **The UI is thin:** panels read engine state and act through `app.run(id, params)`. Colours come from `theme::Tokens`.
- **Plain language in the default UI.** Say "link", "table", "key" and "who can see this", not "endpoint", "schema", "bearer token" or "CORS". Technical terms appear only in Advanced mode and the generated API docs.
- **Rust only** (no handwritten JS/TS). The generated API docs page is rendered from Rust templates. The one exception is Functions (`crates/functions`): the Bun launcher (`runner::LAUNCHER`) and the function templates are short JavaScript/TypeScript kept as strings in Rust, because that's the language functions are written in.
- **Assets:** every icon, image and sample project is original or permissively licensed, with a row in `ATTRIBUTION.md` (author, source, licence). Prefer icons drawn in code (`crates/ui-egui/src/icons.rs`). `cargo xtask assets` enforces it.
- **Quality gates** before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, assets, layers, wit-check, plugins build for `wasm32-wasip2`; the last two report "skipped" until `wit/` and `plugins/` exist). One task id per commit (`M2.1: webhook action plugin`).

## Running and looking at the app
Available now (M0):
- `porchly-cli run --project my.db --cmd 'table.create={"name":"recipes","fields":[{"name":"name","type":"text","required":true}]}' --cmd 'api.key.create={"name":"me","write":true}'` (prints the key once).
- `porchly-cli serve --project my.db [--port 11911]` serves the API on `127.0.0.1`; `porchly-cli commands`, `porchly-cli describe row.list`.
- API reference: `docs/api.md`; machine-readable: `GET /api/openapi.json`.
- **Hit the real API:** `curl -H "Authorization: Bearer $KEY" localhost:11911/api/recipes`. Every API change is checked with real HTTP requests, not only unit tests.

Desktop app (M1):
- `cargo run -p porchly -- --project my.db [--sample] [--no-api]` opens the app; without `--project` it uses `Porchly data/porchly.db` next to the app (portable), or `~/Porchly/porchly.db` when the app's folder can't be written to or data is already there. It turns the API on at start (127.0.0.1, port 11911 or the next free one).
- **Offscreen render** (no window): `cargo run -p porchly-ui-egui --example snapshot -- out.png [--share] [--dark] [--empty] [--fresh-key] [--dialog new_table|add_field|new_key|delete_row]`. Then look at the PNG.
- **Click-through tests:** `crates/ui-egui/tests/ui.rs` (egui_kittest, by button label). Custom-drawn widgets must set `widget_info` so tests and screen readers can find them.
- **Drive the running app:** `porchly --project my.db --control 7979` prints a token; send JSON lines `{"id":1,"method":"auth","params":{"token":"…"}}`, then `engine.execute`, `ui.state`, `ui.select`, `ui.share`, `ui.search`, `ui.screenshot {"path":"/tmp/shot.png"}`. Set `PORCHLY_CONTROL_TOKEN` to choose the token. Docs: `docs/control-protocol.md`; code: `crates/ui-egui/src/control.rs`.
- The app logs to `logs/porchly.log` next to the project (no keys, no row data, never the control token).
- **Porch light** (`docs/porch-light.md`): Cloudflare quick tunnel to a second, read-only-by-default API listener (`porchly_api::Options::public`). `PORCHLY_CLOUDFLARED=/path/to/cloudflared` skips the download; `PORCHLY_TEST_CLOUDFLARED=/path cargo test -p porchly-engine --test porch_light` runs the real end-to-end test (needs the internet); snapshot `--share --porch-light` (with that variable) shows the real On state.
- **Functions** (`docs/functions.md`): JavaScript behind `/api/fn/<name>`, one Bun process per call. `PORCHLY_BUN=/path/to/bun` uses an installed Bun instead of downloading; `PORCHLY_TEST_BUN=/path/to/bun cargo test -p porchly-engine --test functions` runs the end-to-end test (skipped without it, so tests never download). Snapshots: `--functions off|on|public`, `--dialog functions|functions_progress|new_function|turn_off_functions`.
- **Portable builds** (`docs/download.md`): `.github/workflows/release.yml` builds Linux, Windows and macOS packages on a `v*` tag (or by hand: `gh workflow run release.yml`); `.github/workflows/ci.yml` runs `cargo xtask ci` on all three. Windows links the C runtime statically (`.cargo/config.toml`); macOS is a universal `Porchly.app`, signed ad hoc (not notarized).
- `porchly-cli mcp` (M5.3).
- **For UI work, look at the result** (snapshot PNG) and apply the grandpa test: could someone who has never heard of an API finish this task without help?
- Plugins (from M2): `cargo xtask plugins` builds everything in `plugins/`; `porchly-cli plugin install target/plugins/webhook.plugin --project my.db`.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent; edit only the crates you own; delete your target dir when done.

## Roadmap
`ROADMAP.md` (committed) tracks status, milestones and estimates. Update it whenever a milestone task lands. Milestones: M0 core (store, auto REST API, API keys, CLI) → M1 desktop app + tunnel → M2 plugin host + WIT v1 + webhook plugin → M3 automations + email/notification plugins → M4 data-source plugins + native connectors → M5 plugin registry, signing and plugin template.
