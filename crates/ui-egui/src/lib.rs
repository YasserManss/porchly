//! Porchly's desktop interface. Thin by design: panels read engine state and act only through
//! [`PorchlyApp::run`], so everything the UI does is also available to the CLI and to agents.

mod control;
mod dialogs;
mod function_view;
mod grid;
pub mod icons;
mod model;
pub mod names;
mod porch_light;
mod share;
mod sidebar;
mod stats_view;
pub mod theme;

use std::collections::HashMap;

use egui::{RichText, Ui};
use porchly_engine::Engine;
use serde_json::{Value, json};

pub use dialogs::{BackupAction, ChangeItem, Dialog, ImportColumn, PointItem};
use function_view::{Enabling, FnEditor};
use icons::Icon;
use model::{FnItem, KeyInfo, ServerStatus, TableInfo};
use theme::Tokens;

/// Rows shown per page in the grid.
pub(crate) const PAGE_SIZE: u32 = 100;
/// Choices listed in a link dropdown (more are found by searching).
pub(crate) const LINK_CHOICES: u32 = 100;

/// Rows a link dropdown offers, and how many more exist than are listed.
#[derive(Debug, Clone, Default)]
pub(crate) struct LinkChoices {
    pub options: Vec<(i64, String)>,
    pub more: u64,
}

/// A screenshot asked for over the control channel: its id, where to save it, who to answer.
type PendingShot = (u64, Option<String>, std::sync::mpsc::Sender<Result<Value, String>>);

/// How a row is named in link cells and dropdowns: its first text field, else "Row 3".
pub(crate) fn row_label(row: &Value, label_field: Option<&str>, id: i64) -> String {
    label_field
        .and_then(|f| row.get(f))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map_or_else(|| format!("Row {id}"), str::to_owned)
}

/// How often the open Stats window re-reads its numbers, in seconds.
const STATS_EVERY: f64 = 1.0;

/// How often the app checks that today's backup exists, in seconds.
const BACKUP_CHECK_EVERY: f64 = 600.0;

/// How often the API status is re-read while the app is open, in seconds.
const STATUS_EVERY: f64 = 2.0;

pub struct PorchlyApp {
    engine: Engine,
    /// Shown in the top bar (usually the project file name).
    pub project_label: String,
    /// Where the app writes its log, shown to people who need to look into a problem.
    pub log_path: Option<String>,
    pub(crate) tables: Vec<TableInfo>,
    pub(crate) selected: Option<String>,
    pub(crate) rows: Vec<serde_json::Map<String, Value>>,
    pub(crate) total_rows: u64,
    /// Text being typed into cells, by (row id, field).
    pub(crate) edits: HashMap<(i64, String), String>,
    /// The "new row" line under the grid.
    pub(crate) new_row: HashMap<String, String>,
    pub(crate) new_row_flags: HashMap<String, bool>,
    /// Names of the rows chosen in the new row's link dropdowns.
    pub(crate) new_row_link_labels: HashMap<String, String>,
    /// For each table the selected table links to: rows to choose from in link dropdowns.
    pub(crate) link_options: HashMap<String, LinkChoices>,
    /// What was typed in each link dropdown's search box, by linked table.
    pub(crate) link_queries: HashMap<String, String>,
    /// Rows ticked in the grid, by id (for "Delete selected").
    pub(crate) picked_rows: std::collections::BTreeSet<i64>,
    /// The grid's search text.
    pub(crate) search: String,
    /// The first row shown (paging).
    pub(crate) offset: u64,
    pub(crate) server: ServerStatus,
    pub(crate) keys: Vec<KeyInfo>,
    /// Whether the Share panel is open.
    pub show_share: bool,
    pub(crate) dialog: Option<Dialog>,
    /// Why the open dialog's last action failed.
    pub(crate) dialog_error: Option<String>,
    pub(crate) notice: Option<Notice>,
    /// A key just created: shown once, until the person closes it.
    pub(crate) fresh_key: Option<(String, String)>,
    last_status: f64,
    /// When the app last made sure today's backup exists.
    last_backup_check: f64,
    /// When the Stats window's numbers were last read.
    last_stats: f64,
    /// The Stats window shows numbers given to it, not live ones (screenshots).
    stats_frozen: bool,
    control: Option<control::Control>,
    /// Screenshots asked for over the control channel, waiting for the next frame.
    pending_shots: Vec<PendingShot>,
    next_shot: u64,
    /// Whether this project can have functions (it's saved on disk).
    pub(crate) functions_available: bool,
    pub(crate) functions_on: bool,
    pub(crate) functions: Vec<FnItem>,
    /// The function shown instead of a table, if any.
    pub(crate) open_function: Option<FnEditor>,
    /// Turning Functions on (downloading Bun) in the background.
    pub(crate) enabling: Option<Enabling>,
    /// The porch light (the public link).
    pub(crate) light: porch_light::Light,
    pub(crate) light_starting: Option<porch_light::Starting>,
}

pub(crate) struct Notice {
    pub text: String,
    pub error: bool,
    pub until: f64,
    /// Offers "Undo" for the change just made.
    pub undo: bool,
}

impl PorchlyApp {
    pub fn new(engine: Engine, project_label: impl Into<String>) -> Self {
        let mut app = PorchlyApp {
            engine,
            project_label: project_label.into(),
            log_path: None,
            tables: Vec::new(),
            selected: None,
            rows: Vec::new(),
            total_rows: 0,
            edits: HashMap::new(),
            new_row: HashMap::new(),
            new_row_flags: HashMap::new(),
            new_row_link_labels: HashMap::new(),
            link_options: HashMap::new(),
            link_queries: HashMap::new(),
            picked_rows: std::collections::BTreeSet::new(),
            search: String::new(),
            offset: 0,
            server: ServerStatus::default(),
            keys: Vec::new(),
            show_share: false,
            dialog: None,
            dialog_error: None,
            notice: None,
            fresh_key: None,
            last_status: f64::NEG_INFINITY,
            last_backup_check: 0.0,
            last_stats: 0.0,
            stats_frozen: false,
            control: None,
            pending_shots: Vec::new(),
            next_shot: 0,
            functions_available: false,
            functions_on: false,
            functions: Vec::new(),
            open_function: None,
            enabling: None,
            light: porch_light::Light::default(),
            light_starting: None,
        };
        app.refresh_light();
        app.reload_functions();
        app.reload_tables();
        app.reload_keys();
        app.refresh_status();
        app
    }

    /// Installs fonts and colours. Call once when the window opens.
    pub fn setup(ctx: &egui::Context) {
        theme::install(ctx);
    }

    /// Runs an engine command. Errors are shown to the person and return `None`.
    pub fn run(&mut self, id: &str, params: Value) -> Option<Value> {
        match self.engine.execute(id, &params) {
            Ok(v) => Some(v),
            Err(e) => {
                self.error(e.to_string());
                None
            }
        }
    }

    /// Selects a table by name (also used by tests and screenshots).
    pub fn select_table(&mut self, name: &str) {
        if self.selected.as_deref() != Some(name) {
            self.search.clear();
            self.offset = 0;
            self.link_queries.clear();
        }
        self.selected = Some(name.to_owned());
        self.picked_rows.clear();
        self.open_function = None;
        self.edits.clear();
        self.new_row.clear();
        self.new_row_flags.clear();
        self.new_row_link_labels.clear();
        self.reload_rows();
    }

    /// Ticks rows of the open table, as if their checkboxes were clicked (screenshots).
    pub fn pick_rows(&mut self, ids: &[i64]) {
        let shown: Vec<i64> = self.rows.iter().filter_map(|r| r.get("id").and_then(Value::as_i64)).collect();
        self.picked_rows = ids.iter().copied().filter(|id| shown.contains(id)).collect();
    }

    /// Opens a dialog (also used by tests and screenshots).
    pub fn open_dialog(&mut self, dialog: Dialog) {
        self.dialog = Some(dialog);
    }

    /// Starts the control channel on 127.0.0.1:`port` (0 picks a free one). The token comes from
    /// `PORCHLY_CONTROL_TOKEN` when set (at least 16 characters), otherwise it's random. Returns
    /// the port and the token; show the token only to the person who started the app.
    pub fn start_control(&mut self, port: u16, ctx: &egui::Context) -> Result<(u16, String), String> {
        let wanted = std::env::var("PORCHLY_CONTROL_TOKEN").ok();
        let c = control::start(port, wanted, ctx)?;
        let started = (c.port, c.token.clone());
        tracing::info!("control channel listening on 127.0.0.1:{}", c.port);
        self.control = Some(c);
        Ok(started)
    }

    /// Answers control-channel requests (on the UI thread, between frames).
    fn handle_control(&mut self, ctx: &egui::Context) {
        // Screenshots arrive as events a frame after they're asked for.
        let shots: Vec<(u64, std::sync::Arc<egui::ColorImage>)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).map(|id| (*id, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (id, image) in shots {
            if let Some(i) = self.pending_shots.iter().position(|(p, _, _)| *p == id) {
                let (_, path, reply) = self.pending_shots.swap_remove(i);
                let _ = reply.send(control::png(&image, path.as_deref()));
            }
        }
        let Some(c) = &self.control else { return };
        let requests: Vec<control::Request> = c.requests.try_iter().collect();
        for req in requests {
            let answer = self.control_request(ctx, &req);
            if let Some(answer) = answer {
                let _ = req.reply.send(answer);
            }
        }
    }

    /// One control request. `None` means the answer comes later (screenshots).
    fn control_request(&mut self, ctx: &egui::Context, req: &control::Request) -> Option<Result<Value, String>> {
        let p = &req.params;
        let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_owned);
        Some(match req.method.as_str() {
            "engine.execute" => {
                let Some(command) = text("command") else { return Some(Err("\"command\" is missing.".into())) };
                let result =
                    self.engine.execute(&command, p.get("params").unwrap_or(&Value::Null)).map_err(|e| e.to_string());
                // The command may have changed anything the window shows.
                self.reload_functions();
                self.reload_tables();
                self.reload_keys();
                self.refresh_status();
                result
            }
            "ui.state" => Ok(json!({
                "project": self.project_label,
                "tables": self.tables.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
                "selected": self.selected,
                "rows_shown": self.rows.len(),
                "total_rows": self.total_rows,
                "offset": self.offset,
                "search": self.search,
                "share_open": self.show_share,
                "dialog": self.dialog.as_ref().map(|d| format!("{d:?}").split([' ', '{', '(']).next().unwrap_or("").to_owned()),
                "notice": self.notice.as_ref().map(|n| json!({"text": n.text, "error": n.error})),
                "api": {"running": self.server.running, "url": self.server.url},
                "porch_light": {"state": self.light.state, "url": self.light.url, "allow_changes": self.light.allow_changes},
                "functions_on": self.functions_on,
                "functions": self.functions.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
                "open_function": self.open_function.as_ref().map(|f| f.name.clone()),
            })),
            "ui.function" => match text("name") {
                Some(n) if self.functions.iter().any(|f| f.name == n) => {
                    self.open_function(&n);
                    Ok(json!({"open_function": n}))
                }
                _ => Err("\"name\" must name one of the project's functions.".into()),
            },
            "ui.select" => match text("table") {
                Some(t) if self.tables.iter().any(|x| x.name == t) => {
                    self.select_table(&t);
                    Ok(json!({"selected": t}))
                }
                _ => Err("\"table\" must name one of the project's tables.".into()),
            },
            "ui.share" => {
                self.show_share = p.get("open").and_then(Value::as_bool).unwrap_or(!self.show_share);
                Ok(json!({"share_open": self.show_share}))
            }
            "ui.search" => {
                self.set_search(&text("text").unwrap_or_default());
                Ok(json!({"total_rows": self.total_rows}))
            }
            "ui.screenshot" => {
                self.next_shot += 1;
                let id = self.next_shot;
                self.pending_shots.push((id, text("path"), req.reply.clone()));
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(id)));
                ctx.request_repaint();
                return None;
            }
            other => Err(format!(
                "Unknown method \"{}\". Methods: engine.execute, ui.state, ui.select, ui.function, ui.share, ui.search, ui.screenshot.",
                other.chars().take(40).collect::<String>()
            )),
        })
    }

    /// Writes the API guide to a file and opens it in the web browser. The page has no key in
    /// it or in its address, and lives in the system's temporary folder.
    pub fn open_docs(&mut self, ctx: &egui::Context) {
        let Some(html) =
            self.run("api.docs", json!({})).and_then(|v| v.get("html").and_then(Value::as_str).map(str::to_owned))
        else {
            return;
        };
        let path = std::env::temp_dir().join("porchly-api-docs.html");
        if let Err(e) = std::fs::write(&path, html) {
            self.error(format!("Couldn't write the API guide: {e}"));
            return;
        }
        ctx.open_url(egui::OpenUrl::new_tab(file_url(&path)));
        self.info("Opened the API guide in your web browser.");
    }

    /// Starts importing a CSV file: reads it and opens the import dialog with a preview.
    pub fn begin_import(&mut self, path: &std::path::Path) {
        if let Some(preview) = self.run("csv.preview", json!({"path": path}))
            && let Some(dialog) = Dialog::import_csv(path, &preview, self.selected.clone())
        {
            self.open_dialog(dialog);
        }
    }

    /// A file was dropped on the window.
    pub fn file_dropped(&mut self, path: &std::path::Path) {
        let is_table = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| ["csv", "tsv", "txt"].contains(&e.to_ascii_lowercase().as_str()));
        if is_table {
            self.begin_import(path);
        } else {
            self.error("Porchly can import spreadsheets saved as CSV (.csv). Save the file as CSV and drop it again.");
        }
    }

    /// Presses the open dialog's main button, as if clicked (used by screenshots).
    pub fn confirm_dialog(&mut self) {
        dialogs::confirm_now(self);
    }

    /// Shows a just-created key in the Share panel (also used by screenshots).
    pub fn show_new_key(&mut self, name: &str, secret: &str) {
        self.fresh_key = Some((name.to_owned(), secret.to_owned()));
        self.show_share = true;
        self.reload_keys();
    }

    /// Opens "Connect an AI assistant": what to paste into the assistant, for this project.
    pub fn open_connect_ai(&mut self) {
        // The public link when the porch light is on, otherwise this computer's address.
        let base = match (&self.light.url, self.light.is_on(), self.server.url.is_empty()) {
            (Some(url), true, _) => url.clone(),
            (_, _, false) => self.server.url.clone(),
            _ => "http://127.0.0.1:11911".to_owned(),
        };
        let url = format!("{base}/mcp");
        let project = self
            .engine
            .execute("project.info", &Value::Null)
            .ok()
            .and_then(|i| i.get("path").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_else(|| "porchly.db".to_owned());
        // porchly-cli ships next to the app.
        let cli = std::env::current_exe()
            .ok()
            .and_then(|exe| Some(exe.parent()?.join(if cfg!(windows) { "porchly-cli.exe" } else { "porchly-cli" })))
            .filter(|p| p.is_file())
            .map_or_else(|| "porchly-cli".to_owned(), |p| p.display().to_string());
        let pretty = |v: Value| serde_json::to_string_pretty(&v).unwrap_or_default();
        let snippets = vec![
            (
                "Claude Code (run this in a terminal)".to_owned(),
                format!("claude mcp add --transport http porchly {url} --header \"Authorization: Bearer YOUR_KEY\""),
            ),
            (
                "Cursor, VS Code and others that connect to an address".to_owned(),
                pretty(json!({"mcpServers": {"porchly": {
                    "type": "http", "url": url, "headers": {"Authorization": "Bearer YOUR_KEY"}}}})),
            ),
            (
                "Claude Desktop and others that start a program".to_owned(),
                pretty(json!({"mcpServers": {"porchly": {"command": cli, "args": ["mcp", "--project", project]}}})),
            ),
        ];
        self.open_dialog(Dialog::ConnectAi { snippets, copied: None });
    }

    /// Opens the Settings dialog with the current settings.
    pub fn open_settings(&mut self) {
        if let Some(current) = self.run("settings.get", json!({})) {
            let d = Dialog::settings(&current, self.functions_on);
            self.open_dialog(d);
        }
    }

    /// Re-reads the API's status now (screenshots).
    pub fn refresh_status_now(&mut self) {
        self.refresh_status();
    }

    /// Turns the API on (only reachable from this computer).
    pub fn start_api(&mut self) {
        if let Some(v) = self.run("api.start", json!({})) {
            self.server = ServerStatus::from(&v);
        }
    }

    pub(crate) fn reload_tables(&mut self) {
        let tables = self.run("table.list", json!({})).and_then(|v| serde_json::from_value::<Vec<TableInfo>>(v).ok());
        self.tables = tables.unwrap_or_default();
        let still_there = self.selected.as_ref().is_some_and(|s| self.tables.iter().any(|t| &t.name == s));
        if !still_there {
            self.selected = self.tables.first().map(|t| t.name.clone());
        }
        self.reload_rows();
    }

    pub(crate) fn reload_rows(&mut self) {
        self.rows.clear();
        self.total_rows = 0;
        self.reload_link_options();
        let Some(table) = self.selected.clone() else { return };
        let links: Vec<String> = self
            .selected_table()
            .map(|t| t.fields.iter().filter(|f| f.links_to.is_some()).map(|f| f.name.clone()).collect())
            .unwrap_or_default();
        let mut params = json!({"table": table, "limit": PAGE_SIZE, "offset": self.offset});
        if let Some(p) = params.as_object_mut() {
            if !self.search.trim().is_empty() {
                p.insert("search".into(), json!(self.search.trim()));
            }
            if !links.is_empty() {
                // Linked rows come along, so link cells can show names.
                p.insert("expand".into(), json!(links.join(",")));
            }
        }
        if let Some(page) = self.run("row.list", params) {
            self.total_rows = page.get("total").and_then(Value::as_u64).unwrap_or(0);
            self.rows = page
                .get("items")
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(|r| r.as_object().cloned()).collect())
                .unwrap_or_default();
            // Only rows on screen can be ticked: nothing out of sight is ever deleted.
            let shown: std::collections::BTreeSet<i64> =
                self.rows.iter().filter_map(|r| r.get("id").and_then(Value::as_i64)).collect();
            self.picked_rows.retain(|id| shown.contains(id));
            // Past the end (rows were deleted elsewhere): go back to the last page.
            if self.rows.is_empty() && self.offset > 0 && self.total_rows > 0 {
                self.offset = (self.total_rows - 1) / u64::from(PAGE_SIZE) * u64::from(PAGE_SIZE);
                self.reload_rows();
            }
        }
    }

    /// The text field used to name a table's rows (its first text field).
    pub(crate) fn label_field(&self, table: &str) -> Option<String> {
        self.tables
            .iter()
            .find(|t| t.name == table)
            .and_then(|t| t.fields.iter().find(|f| f.kind == porchly_core::FieldKind::Text))
            .map(|f| f.name.clone())
    }

    /// Loads the rows that link fields of the selected table can point to: the first
    /// [`LINK_CHOICES`], or those matching what was typed in the dropdown's search box.
    fn reload_link_options(&mut self) {
        self.link_options.clear();
        let targets: Vec<String> = self
            .selected_table()
            .map(|t| t.fields.iter().filter_map(|f| f.links_to.clone()).collect())
            .unwrap_or_default();
        for target in targets {
            if self.link_options.contains_key(&target) {
                continue;
            }
            let label_field = self.label_field(&target);
            let mut params = json!({"table": target, "limit": LINK_CHOICES});
            if let (Some(p), Some(q)) = (params.as_object_mut(), self.link_queries.get(&target))
                && !q.trim().is_empty()
            {
                p.insert("search".into(), json!(q.trim()));
            }
            let page = self.run("row.list", params);
            let total = page.as_ref().and_then(|p| p.get("total")).and_then(Value::as_u64).unwrap_or(0);
            let rows = page.and_then(|p| p.get("items").and_then(Value::as_array).cloned()).unwrap_or_default();
            let options: Vec<(i64, String)> = rows
                .iter()
                .filter_map(|r| {
                    let id = r.get("id")?.as_i64()?;
                    Some((id, row_label(r, label_field.as_deref(), id)))
                })
                .collect();
            let more = total.saturating_sub(options.len() as u64);
            self.link_options.insert(target, LinkChoices { options, more });
        }
    }

    /// Sets the search text of a link dropdown and reloads its choices.
    pub(crate) fn set_link_query(&mut self, target: &str, text: String) {
        self.link_queries.insert(target.to_owned(), text);
        self.reload_link_options();
    }

    /// Sets the grid's search text and goes back to the first page.
    pub fn set_search(&mut self, text: &str) {
        self.search = text.to_owned();
        self.offset = 0;
        self.reload_rows();
    }

    /// Shows another page of rows.
    pub(crate) fn go_to_page(&mut self, offset: u64) {
        self.offset = offset;
        self.edits.clear();
        self.reload_rows();
    }

    pub(crate) fn reload_keys(&mut self) {
        let keys = self.run("api.key.list", json!({})).and_then(|v| serde_json::from_value::<Vec<KeyInfo>>(v).ok());
        self.keys = keys.unwrap_or_default();
    }

    pub(crate) fn refresh_status(&mut self) {
        if let Some(v) = self.run("api.status", json!({})) {
            self.server = ServerStatus::from(&v);
        }
    }

    pub(crate) fn selected_table(&self) -> Option<&TableInfo> {
        let name = self.selected.as_ref()?;
        self.tables.iter().find(|t| &t.name == name)
    }

    pub(crate) fn info(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice { text: text.into(), error: false, until: 0.0, undo: false });
    }

    /// A notice with an Undo button, for a change that was just made to rows.
    pub(crate) fn info_with_undo(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice { text: text.into(), error: false, until: 0.0, undo: true });
    }

    /// Undoes the newest change (or the change `id`) and shows what came back.
    pub fn undo(&mut self, id: Option<i64>) {
        if let Some(done) = self.run("undo.run", json!({"id": id})) {
            let what = done.get("summary").and_then(Value::as_str).unwrap_or("the change").to_owned();
            self.reload_tables();
            self.info(format!("Undone: {}.", lower_first(&what)));
        }
    }

    /// Opens the Stats window. It refreshes itself while open.
    pub fn open_stats(&mut self) {
        if let Some(data) = self.run("stats.get", json!({})) {
            self.open_dialog(Dialog::Stats { data });
        }
    }

    /// Shows the Stats window with these numbers and leaves them alone (screenshots).
    pub fn show_stats(&mut self, data: Value) {
        self.stats_frozen = true;
        self.open_dialog(Dialog::Stats { data });
    }

    /// Keeps the Stats window's numbers current.
    fn refresh_stats(&mut self, ctx: &egui::Context, now: f64) {
        if !matches!(self.dialog, Some(Dialog::Stats { .. })) {
            self.stats_frozen = false;
            return;
        }
        if self.stats_frozen {
            return;
        }
        if now - self.last_stats >= STATS_EVERY {
            self.last_stats = now;
            if let (Ok(fresh), Some(Dialog::Stats { data })) =
                (self.engine.execute("stats.get", &Value::Null), &mut self.dialog)
            {
                *data = fresh;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(STATS_EVERY));
    }

    /// Opens the Backups window: recent changes to undo, and backups to restore.
    pub fn open_backups(&mut self) {
        let changes = self.run("undo.list", json!({"limit": 30})).unwrap_or_default();
        let points = self.run("backup.list", json!({})).unwrap_or_default();
        self.open_dialog(Dialog::backups(&changes, &points));
    }

    pub(crate) fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!("{text}");
        self.notice = Some(Notice { text, error: true, until: 0.0, undo: false });
    }

    /// Draws the whole window.
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        if now - self.last_status > STATUS_EVERY {
            self.last_status = now;
            self.refresh_status();
            self.refresh_light();
            // Apps may have changed the data through the API: show it, unless the person
            // is in the middle of typing or answering a dialog.
            let busy = !self.edits.is_empty() || self.dialog.is_some() || ctx.memory(|m| m.focused().is_some());
            if !busy {
                self.reload_tables();
                self.reload_functions();
                self.reload_function_log();
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(STATUS_EVERY));
        // The app may stay open for days: make sure each day gets its backup.
        if now - self.last_backup_check > BACKUP_CHECK_EVERY {
            self.last_backup_check = now;
            if let Err(e) = self.engine.execute("backup.daily", &Value::Null) {
                tracing::warn!("couldn't make today's backup: {e}");
            }
        }

        self.handle_control(&ctx);
        self.poll_function_jobs(&ctx);
        self.refresh_stats(&ctx, now);
        self.poll_light(&ctx);
        let t = Tokens::get(&ctx);
        self.top_bar(ui, &t);
        self.notice_bar(ui, &t, now);
        if self.show_share {
            share::show(self, ui, &t);
        }
        sidebar::show(self, ui, &t);
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.bg).inner_margin(20)).show(ui, |ui| {
            if self.open_function.is_some() {
                function_view::show(self, ui, &t);
            } else {
                grid::show(self, ui, &t);
            }
        });
        dialogs::show(self, &ctx);
    }

    fn top_bar(&mut self, ui: &mut Ui, t: &Tokens) {
        egui::Panel::top("top_bar")
            .exact_size(52.0)
            .frame(
                egui::Frame::NONE
                    .fill(t.chrome)
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .stroke(egui::Stroke::new(1.0, t.border)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(24.0), egui::Sense::hover());
                    icons::paint(ui.painter(), rect, Icon::Lamp, t.accent);
                    ui.label(RichText::new("Porchly").size(19.0).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new(&self.project_label).color(t.text_dim));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Some(Icon::Gear), "Settings", false).clicked() {
                            self.open_settings();
                        }
                        ui.add_space(8.0);
                        if icons::button(ui, Some(Icon::Stats), "Stats", false)
                            .on_hover_text("How much your tables are used, and how big they are")
                            .clicked()
                        {
                            self.open_stats();
                        }
                        ui.add_space(8.0);
                        if icons::button(ui, Some(Icon::History), "Backups", false)
                            .on_hover_text("Undo recent changes, or go back to a backup")
                            .clicked()
                        {
                            self.open_backups();
                        }
                        ui.add_space(8.0);
                        let label = if self.show_share { "Close sharing" } else { "Share" };
                        if icons::button(ui, Some(Icon::Share), label, !self.show_share).clicked() {
                            self.show_share = !self.show_share;
                            if self.show_share {
                                self.reload_keys();
                            }
                        }
                        ui.add_space(12.0);
                        if self.light.is_on() {
                            icons::status_dot(ui, true, "API on, porch light on");
                        } else if self.server.running {
                            icons::status_dot(ui, true, "API on (this computer only)");
                        } else {
                            icons::status_dot(ui, false, "API off");
                        }
                    });
                });
            });
    }

    fn notice_bar(&mut self, ui: &mut Ui, t: &Tokens, now: f64) {
        if let Some(notice) = &mut self.notice {
            if notice.until == 0.0 {
                // Longer when there's something to decide.
                notice.until = now + if notice.error || notice.undo { 12.0 } else { 4.0 };
            }
            if now > notice.until {
                self.notice = None;
            }
        }
        let Some(notice) = &self.notice else {
            // The bar's place is kept (at no height) when there's nothing to say. Widgets
            // get their ids from what was drawn before them, so a bar that came and went
            // would change the id of everything after it: a click made at that moment would
            // be lost, and a box being typed in would lose the cursor.
            egui::Panel::bottom("notice")
                .exact_size(0.0)
                .frame(egui::Frame::NONE)
                .show_separator_line(false)
                .show(ui, |_| {});
            return;
        };
        let (fg, bg) = if notice.error { (t.danger, t.danger_soft) } else { (t.ok, t.ok_soft) };
        let text = notice.text.clone();
        let offer_undo = notice.undo;
        let mut undo = false;
        let mut close = false;
        egui::Panel::bottom("notice")
            .show_separator_line(false)
            .frame(egui::Frame::NONE.fill(bg).inner_margin(egui::Margin::symmetric(16, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(text).color(fg));
                    if offer_undo {
                        undo = icons::button(ui, None, "Undo", false).clicked();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close = icons::icon_button(ui, Icon::Close, "Dismiss").clicked();
                    });
                });
            });
        if close {
            self.notice = None;
        }
        if undo {
            self.undo(None);
        }
    }
}

/// "Deleted 3 rows" → "deleted 3 rows", for use inside a sentence.
fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `file:///…` for a local path (Windows paths get forward slashes).
fn file_url(path: &std::path::Path) -> String {
    let p = path.display().to_string().replace('\\', "/");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn file_urls() {
        assert_eq!(super::file_url(std::path::Path::new("/tmp/a.html")), "file:///tmp/a.html");
        assert_eq!(super::file_url(std::path::Path::new("C:\\Temp\\a.html")), "file:///C:/Temp/a.html");
    }
}
