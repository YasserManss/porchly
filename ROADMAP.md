# Roadmap

| Milestone | What you get | Status |
|---|---|---|
| **M0 Core** | Project file (SQLite), tables and fields, an API generated from the tables, keys, snapshots, `porchly-cli` | ✅ Done |
| **M1 Desktop app + public link** | Double-click app with a spreadsheet-like table view, links between tables, a Share panel for keys, the "porch light" public link, CSV import/export, functions (JavaScript behind the API), settings, portable downloads for Windows, macOS and Linux | ✅ Done (v0.1.0) |
| **M2 Plugins** | WASM plugin host (wasmtime), the plugin interface (WIT v1), custom endpoints built from blocks, a webhook plugin | Planned |
| **M3 Automations** | "When a row is added → send an email" rules, email (SMTP) and notification (ntfy) plugins | Planned |
| **M4 External data** | Postgres and MySQL tables, REST APIs as data sources | Planned |
| **M5 Ecosystem** | Plugin registry, signed plugins, plugin template | Planned |

## M0 — Core ✅
- [x] M0.0 Workspace, lints, `cargo xtask ci` (fmt, clippy, tests, assets, layers)
- [x] M0.1 Errors, validated names, field types, limits (`porchly-core`)
- [x] M0.2 Project file: pragmas, metadata, format version, migrations
- [x] M0.3 Tables and fields; snapshots before destructive changes
- [x] M0.4 Rows: create, read, update, delete, paging, sorting, filters
- [x] M0.5 Keys: create (shown once), list, turn off, verify (hashed)
- [x] M0.6 Command engine with panic guard; every command fuzzed with hostile parameters
- [x] M0.7 HTTP API: keys, read/write permission, JSON errors, size limits, rate limit
- [x] M0.8 `porchly-cli`: `serve`, `run`, `commands`, `describe`
- [x] M0.9 Security test suite (`crates/api/tests/security.rs`)
- [x] M0.10 `GET /api/openapi.json`, `docs/api.md`

## M1 — Desktop app + public link ✅
- [x] M1.1 App shell (egui): tables sidebar, grid view, theme and icons drawn in code
- [x] M1.2 Edit rows and fields in the grid; new-table dialog
- [x] M1.3 Share panel: API on/off, keys, example requests
- [x] M1.3a Friendly port errors
- [x] M1.3b Automatic port fallback when 11911 is busy
- [x] M1.9 Links between tables: link fields, `expand`, conditions and sorting on linked fields, link pickers in the app
- [x] M1.10 Change fields: rename, change type (values converted or the change is refused, naming rows), required on/off, link target; clickable column headers with Change/Delete
- [x] M1.11 Functions: JavaScript/TypeScript behind API routes (`/api/fn/…`), run with Bun, one process per call; opt-in, greyed out until turned on
  - [x] M1.11a Turn on Functions (downloads Bun once, after asking)
  - [x] M1.11b Functions folder, editor, access setting (key / no key)
  - [x] M1.11c The route: one Bun process per call
  - [x] M1.11d Logs, test panel, commands, docs
- [x] M1.4 Porch light: Cloudflare quick tunnel, status, read-only by default
- [x] M1.12 Settings: requests per app, function time limit, port, start the API on open
- [x] M1.17 Stats: requests, rows and size at a glance; activity in the last hour; use per key, table and function; updates by itself
- [x] M1.16 Your own address: a stable public link through your own Cloudflare tunnel; its token kept in the system keychain
- [x] M1.15 AI assistants (MCP): `/mcp` with the same keys, `porchly-cli mcp`, tools for rows (and tables when allowed), "Connect an AI assistant" in Share
- [x] M1.14 Backups and undo: row changes and deletes can be undone for a week; a backup every day (with the functions' code); going back to a backup or snapshot in one click
- [x] M1.13 Many rows at once: add a list of rows (all or nothing), change or delete every row matching conditions; tick rows in the app and delete them together
- [x] M1.5 Control channel (token-protected, 127.0.0.1) and screenshots; offscreen screenshots
- [x] M1.6 CSV import/export (preview with type guesses, all-or-nothing, drag a file onto the window)
- [x] M1.7 Sample project and first-run welcome
- [x] M1.8 Portable app for macOS, Windows and Linux: one file, no installer, no admin rights; data next to the app

## Ideas (not scheduled)
- **Saved queries:** read-only queries written in the app, each with its own address (`/api/queries/{name}`), for totals, counts and reports. Callers pass values, never SQL. An open raw-SQL endpoint was considered and rejected: it would bypass validation, key permissions and snapshots, and expose Porchly's own tables and files outside the project.
- **SQL console in the app:** for the owner only, read-only, never reachable over the API.
