//! The command engine. Every user-visible action in Porchly is a [`Command`]; the GUI, CLI,
//! control channel and MCP server all go through [`Engine::execute`].

pub mod cmd;
mod net;
mod params;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::{Arc, MutexGuard};

use porchly_core::{Error, Result};
use porchly_functions::Service;
use porchly_store::{Shared, Store, lock};
use porchly_tunnel::PorchLight;
use serde_json::Value;

pub use cmd::{Command, registry};
use net::Net;

/// Runs commands against one project. Cheap to clone; clones share the project and the server.
#[derive(Clone)]
pub struct Engine {
    store: Shared,
    net: Arc<Net>,
    functions: Arc<Service>,
    light: Arc<Light>,
}

/// The porch light, and whether changes are allowed through it.
pub(crate) struct Light {
    pub tunnel: PorchLight,
    pub allow_changes: Arc<std::sync::atomic::AtomicBool>,
    /// Where the token of the owner's own tunnel is kept: the system keychain, found on
    /// first use (or memory, for previews and tests).
    secrets: std::sync::OnceLock<Arc<dyn porchly_secrets::Secrets>>,
    /// Never touch the real keychain (previews and tests).
    memory_only: bool,
}

impl Light {
    pub(crate) fn secrets(&self) -> &Arc<dyn porchly_secrets::Secrets> {
        self.secrets.get_or_init(|| {
            if self.memory_only { Arc::new(porchly_secrets::Memory::default()) } else { porchly_secrets::system() }
        })
    }
}

/// What a command can reach. The project is locked only when a command first asks for it,
/// so commands that don't touch the data (starting or stopping the server) never hold the
/// lock that API requests need.
pub struct Ctx<'a> {
    shared: &'a Shared,
    guard: Option<MutexGuard<'a, Store>>,
    net: &'a Net,
    functions: &'a Arc<Service>,
    light: &'a Light,
}

impl<'a> Ctx<'a> {
    pub(crate) fn store(&mut self) -> &mut Store {
        let shared = self.shared;
        self.guard.get_or_insert_with(|| lock(shared))
    }

    pub(crate) fn shared(&self) -> &Shared {
        self.shared
    }

    pub(crate) fn net(&self) -> &'a Net {
        self.net
    }

    pub(crate) fn functions(&self) -> &Arc<Service> {
        self.functions
    }

    pub(crate) fn light(&self) -> &'a Light {
        self.light
    }

    /// Releases the project lock, if held.
    pub(crate) fn unlock(&mut self) {
        self.guard = None;
    }
}

impl Engine {
    /// Opens (or creates) the project at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self::new(Store::open(path)?.into_shared()))
    }

    pub fn new(store: Shared) -> Self {
        Self::with_net(store, true)
    }

    /// An engine that never opens network ports (`api.start` fails). For previews and tests.
    pub fn new_offline(store: Shared) -> Self {
        Self::with_net(store, false)
    }

    fn with_net(store: Shared, allowed: bool) -> Self {
        let functions = Arc::new(functions_service(&store));
        let light = Arc::new(light_for(&store, !allowed));
        let net = Arc::new(Net::new(allowed));
        cmd::apply_settings(&lock(&store), &net, &functions);
        let engine = Engine { store, net, functions, light };
        // Today's backup, if there isn't one yet. Opening a project must never fail over it.
        let _ = engine.execute("backup.daily", &Value::Null);
        engine
    }

    /// What the API serves besides the tables (functions and their internal key), for
    /// servers started outside the engine (`porchly-cli serve`). Tell the engine the
    /// server's address with [`Engine::api_started`].
    pub fn api_options(&self) -> porchly_api::Options {
        cmd::api_options(&self.functions, self.net.limits(), self.net.stats())
    }

    /// Runs functions with this Bun program instead of Porchly's own copy (tests, `--bun`).
    pub fn use_bun(&self, path: Option<std::path::PathBuf>) {
        self.functions.use_bun(path);
    }

    /// Keeps secrets in memory instead of the system keychain (tests). Call before any
    /// command that uses a secret.
    pub fn use_memory_secrets(&self) {
        let _ = self.light.secrets.set(Arc::new(porchly_secrets::Memory::default()));
    }

    /// Uses this cloudflared instead of Porchly's own copy (tests).
    pub fn use_cloudflared(&self, path: Option<std::path::PathBuf>) {
        self.light.tunnel.use_program(path);
    }

    /// Records where a server started outside the engine listens, for functions.
    pub fn api_started(&self, url: &str) {
        self.functions.set_api_url(Some(url.to_owned()));
    }

    /// The store, for sharing with an HTTP server started outside the engine.
    pub fn store(&self) -> Shared {
        self.store.clone()
    }

    /// Runs a command. Writes happen in transactions, so an error or an escaped panic
    /// leaves the project as it was. The panic guard is a safety net, not a licence.
    pub fn execute(&self, id: &str, params: &Value) -> Result<Value> {
        let cmd = cmd::find(id).ok_or_else(|| {
            Error::not_found(format!("There's no command \"{}\".", id.chars().take(60).collect::<String>()))
        })?;
        if !(cmd.enabled)(&lock(&self.store)) {
            return Err(Error::invalid(format!("\"{}\" isn't available right now.", cmd.label)));
        }
        let mut cx =
            Ctx { shared: &self.store, guard: None, net: &self.net, functions: &self.functions, light: &self.light };
        match catch_unwind(AssertUnwindSafe(|| (cmd.run)(&mut cx, params))) {
            Ok(result) => result,
            Err(_) => {
                Err(Error::Internal(format!("the command \"{id}\" failed unexpectedly; your data was not changed")))
            }
        }
    }
}

/// Functions for the project in `store`: where it lives, whether they're on, and a fresh
/// internal key (never stored; a new one every time Porchly starts).
fn functions_service(store: &Shared) -> Service {
    let s = lock(store);
    let dir = s.path().and_then(Path::parent).map(Path::to_path_buf);
    let enabled = s.setting(cmd::FUNCTIONS_SETTING).ok().flatten().as_deref() == Some("on");
    let mut bytes = [0u8; 32];
    // Without randomness there is no safe key; an empty one is refused by the API.
    let token = match getrandom::fill(&mut bytes) {
        Ok(()) => format!("pf_{}", data_encoding::BASE32_NOPAD.encode(&bytes).to_ascii_lowercase()),
        Err(_) => String::new(),
    };
    Service::new(dir, enabled, token)
}

/// The porch light for the project in `store`. Always off at start; whether changes are
/// allowed through it is remembered.
fn light_for(store: &Shared, memory_only: bool) -> Light {
    let s = lock(store);
    let dir = s.path().and_then(Path::parent);
    let allow = s.setting(cmd::PORCH_LIGHT_CHANGES).ok().flatten().as_deref() == Some("on");
    Light {
        tunnel: PorchLight::new(dir),
        allow_changes: Arc::new(std::sync::atomic::AtomicBool::new(allow)),
        secrets: std::sync::OnceLock::new(),
        memory_only,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine() -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::open(dir.path().join("p.db")).unwrap();
        (dir, engine)
    }

    #[test]
    fn end_to_end() {
        let (_dir, e) = engine();
        e.execute(
            "table.create",
            &json!({"name":"recipes","fields":[{"name":"name","type":"text","required":true},{"name":"minutes","type":"integer"}]}),
        )
        .unwrap();
        let row = e.execute("row.insert", &json!({"table":"recipes","values":{"name":"Soup","minutes":30}})).unwrap();
        assert_eq!(row["id"], 1);
        let page = e.execute("row.list", &json!({"table":"recipes","where":{"minutes":30}})).unwrap();
        assert_eq!(page["total"], 1);
        let page = e.execute("row.list", &json!({"table":"recipes","where":{"minutes":"lt.30"}})).unwrap();
        assert_eq!(page["total"], 0);
        let page = e.execute("row.list", &json!({"table":"recipes","where":{"minutes":null}})).unwrap();
        assert_eq!(page["total"], 0, "null means no value");
        let key = e.execute("api.key.create", &json!({"name":"site"})).unwrap();
        assert!(key["secret"].as_str().unwrap().starts_with("pk_"));
        assert_eq!(key["key"]["can_write"], false);
        e.execute("table.drop", &json!({"name":"recipes"})).unwrap();
        assert_eq!(e.execute("snapshot.list", &json!({})).unwrap().as_array().unwrap().len(), 1);
    }

    #[test]
    fn unknown_command() {
        let (_dir, e) = engine();
        assert!(matches!(e.execute("nope.nope", &json!({})), Err(Error::NotFound(_))));
    }

    #[test]
    fn sample_project() {
        let (_dir, e) = engine();
        e.execute("sample.load", &json!({})).unwrap();
        let page = e.execute("row.list", &json!({"table":"recipes"})).unwrap();
        assert!(page["total"].as_u64().unwrap() >= 5);
        assert!(e.execute("sample.load", &json!({})).is_err(), "loading twice is refused");
        assert!(e.execute("sample.load", &json!({"name":"nope"})).is_err());
    }

    #[test]
    fn links_through_commands() {
        let (_dir, e) = engine();
        e.execute("sample.load", &json!({"name": "orders"})).unwrap();
        let page = e
            .execute(
                "row.list",
                &json!({"table": "orders", "expand": "customer", "where": {"customer.city": "Lisbon"}}),
            )
            .unwrap();
        assert_eq!(page["total"], 3);
        assert_eq!(page["items"][0]["expand"]["customer"]["name"], "Ana Lima");
        let row = e.execute("row.get", &json!({"table": "orders", "id": 2, "expand": "customer"})).unwrap();
        assert_eq!(row["expand"]["customer"]["city"], "Leeds");
        e.execute(
            "field.add",
            &json!({"table": "customers", "name": "referred_by", "type": "link", "links_to": "customers"}),
        )
        .unwrap();
        assert!(
            e.execute("field.add", &json!({"table": "orders", "name": "x", "type": "link"})).is_err(),
            "needs a target"
        );
        assert!(e.execute("table.drop", &json!({"name": "customers"})).is_err(), "orders link to it");
    }

    #[test]
    fn csv_round_trip() {
        let (dir, e) = engine();
        let file = dir.path().join("Guest List.csv");
        std::fs::write(&file, "Name,Guests,Vegetarian,Arrives\nAnn,2,yes,2026-10-10\nBob,1,no,\n").unwrap();
        let preview = e.execute("csv.preview", &json!({"path": file})).unwrap();
        assert_eq!(preview["suggested_table"], "guest_list");
        assert_eq!(preview["total_rows"], 2);
        assert_eq!(preview["columns"][1]["type"], "integer");
        let done = e.execute("csv.import", &json!({"path": file, "table": "guest_list"})).unwrap();
        assert_eq!((done["imported"].as_u64(), done["created"].as_bool()), (Some(2), Some(true)));
        let row = e.execute("row.get", &json!({"table": "guest_list", "id": 1})).unwrap();
        assert_eq!((row["guests"].as_i64(), row["vegetarian"].as_bool()), (Some(2), Some(true)));

        // Adding the same file again goes into the existing table.
        e.execute("csv.import", &json!({"path": file, "table": "guest_list"})).unwrap();
        assert_eq!(e.execute("row.list", &json!({"table": "guest_list"})).unwrap()["total"], 4);

        // Export, then import the export into a new table: ids and timestamps are skipped.
        let out = dir.path().join("out.csv");
        assert_eq!(e.execute("csv.export", &json!({"table": "guest_list", "path": out})).unwrap()["rows"], 4);
        assert!(e.execute("csv.export", &json!({"table": "guest_list", "path": out})).is_err(), "won't overwrite");
        let again = e.execute("csv.import", &json!({"path": out, "table": "copy"})).unwrap();
        assert_eq!(again["imported"], 4);
        assert_eq!(again["skipped_columns"], json!(["id", "created_at", "updated_at"]));
    }

    #[test]
    fn csv_import_is_all_or_nothing() {
        let (dir, e) = engine();
        let file = dir.path().join("bad.csv");
        std::fs::write(&file, "name,count\na,1\nb,2\n").unwrap();
        e.execute("csv.import", &json!({"path": file, "table": "things"})).unwrap();
        std::fs::write(&file, "name,count\nc,3\nd,lots\n").unwrap();
        let err = e.execute("csv.import", &json!({"path": file, "table": "things"})).unwrap_err();
        assert!(err.to_string().contains("Line 3, column \"count\""), "{err}");
        assert_eq!(e.execute("row.list", &json!({"table": "things"})).unwrap()["total"], 2, "nothing added");
        // A failed import into a new table leaves no table behind.
        assert!(
            e.execute("csv.import", &json!({"path": file, "table": "fresh", "types": {"count": "integer"}})).is_err()
        );
        assert!(e.execute("row.list", &json!({"table": "fresh"})).is_err());
        // Unknown columns, wrong file types.
        std::fs::write(&file, "name,colour\nx,red\n").unwrap();
        assert!(
            e.execute("csv.import", &json!({"path": file, "table": "things"}))
                .unwrap_err()
                .to_string()
                .contains("colour")
        );
        let txt = dir.path().join("secret.key");
        std::fs::write(&txt, "a,b\n1,2\n").unwrap();
        assert!(e.execute("csv.preview", &json!({"path": txt})).is_err(), "only .csv/.tsv/.txt");
    }

    #[test]
    fn fields_can_be_changed() {
        let (_dir, e) = engine();
        e.execute("sample.load", &json!({})).unwrap();
        e.execute("field.update", &json!({"table": "recipes", "name": "minutes", "new_name": "cooking_time"})).unwrap();
        e.execute("field.update", &json!({"table": "recipes", "name": "serves", "type": "text", "required": true}))
            .unwrap();
        let fields = e.execute("table.list", &json!({})).unwrap()[0]["fields"].clone();
        assert_eq!(fields[1]["name"], "cooking_time");
        assert_eq!((fields[2]["type"].as_str(), fields[2]["required"].as_bool()), (Some("text"), Some(true)));
        let row = e.execute("row.get", &json!({"table": "recipes", "id": 1})).unwrap();
        assert_eq!(row["serves"], "4");
        assert!(e.execute("field.update", &json!({"table": "recipes", "name": "name", "type": "integer"})).is_err());
        assert!(e.execute("field.update", &json!({"table": "recipes", "name": "name", "new_name": "sort"})).is_err());
    }

    #[test]
    fn server_starts_serves_and_stops() {
        let (_dir, e) = engine();
        e.execute("sample.load", &json!({})).unwrap();
        let secret = e.execute("api.key.create", &json!({"name":"t"})).unwrap()["secret"].as_str().unwrap().to_owned();
        let status = e.execute("api.start", &json!({})).unwrap();
        assert_eq!(status["running"], true);
        let port = status["port"].as_u64().unwrap();
        assert!(port >= u64::from(porchly_api::DEFAULT_PORT), "never a low port");

        // A real HTTP request while the engine is idle.
        let body = http_get(port as u16, "/api/recipes?limit=1", &secret);
        assert!(body.contains("\"total\""), "{body}");

        // Starting again is harmless and keeps the same address.
        assert_eq!(e.execute("api.start", &json!({})).unwrap()["port"], port);
        let status = e.execute("api.stop", &json!({})).unwrap();
        assert_eq!(status["running"], false);
        assert_eq!(e.execute("api.status", &json!({})).unwrap()["running"], false);
    }

    #[test]
    fn settings_take_effect_and_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.db");
        let e = Engine::open(&path).unwrap();
        let s = e.execute("settings.get", &json!({})).unwrap();
        assert_eq!((s["requests_per_second"].as_u64(), s["function_timeout"].as_u64()), (Some(20), Some(0)));
        assert_eq!((s["port"].as_u64(), s["start_api"].as_bool()), (Some(11911), Some(true)));
        assert_eq!(s["ai_can_make_tables"], false, "off unless the owner says so");
        e.execute("settings.set", &json!({"ai_can_make_tables": true})).unwrap();
        assert_eq!(e.execute("settings.get", &json!({})).unwrap()["ai_can_make_tables"], true);
        assert_eq!(e.net.limits().per_second(), 20);

        e.execute("settings.set", &json!({"requests_per_second": 100, "function_timeout": 30})).unwrap();
        assert_eq!(e.net.limits().per_second(), 100, "live");
        assert_eq!(e.functions.timeout(), Some(std::time::Duration::from_secs(30)));

        // A bad value changes nothing at all.
        for bad in [
            json!({"requests_per_second": 5, "port": 80}),
            json!({"requests_per_second": 0}),
            json!({"requests_per_second": 1001}),
            json!({"requests_per_second": "lots"}),
            json!({"function_timeout": 7}),
            json!({"start_api": "yes"}),
        ] {
            assert!(e.execute("settings.set", &bad).is_err(), "{bad}");
        }
        assert_eq!(e.execute("settings.get", &json!({})).unwrap()["requests_per_second"], 100);

        // A new port moves the running API.
        e.execute("api.start", &json!({})).unwrap();
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let out = e.execute("settings.set", &json!({"port": free, "start_api": false})).unwrap();
        assert_eq!(out["api"]["port"], free);
        assert_eq!(e.execute("api.status", &json!({})).unwrap()["port"], free);
        e.execute("api.stop", &json!({})).unwrap();
        drop(e);

        // Kept with the project, and applied when it opens again.
        let e = Engine::open(&path).unwrap();
        let s = e.execute("settings.get", &json!({})).unwrap();
        assert_eq!((s["port"].as_u64(), s["start_api"].as_bool()), (Some(u64::from(free)), Some(false)));
        assert_eq!(e.net.limits().per_second(), 100);
        assert_eq!(e.functions.timeout(), Some(std::time::Duration::from_secs(30)));
    }

    #[test]
    fn bulk_row_commands() {
        let (_dir, e) = engine();
        e.execute("sample.load", &json!({"name": "orders"})).unwrap();
        let added = e
            .execute(
                "row.insert_many",
                &json!({"table": "orders", "rows": [{"item": "Tea"}, {"item": "Jam", "quantity": 2}]}),
            )
            .unwrap();
        assert_eq!((added["count"].as_u64(), added["items"][1]["item"].as_str()), (Some(2), Some("Jam")));
        assert!(
            e.execute("row.insert_many", &json!({"table": "orders", "rows": [{"item": "Ok"}, {"quantity": 1}]}))
                .is_err()
        );
        assert!(e.execute("row.insert_many", &json!({"table": "orders", "rows": [5]})).is_err());
        assert_eq!(e.execute("row.list", &json!({"table": "orders"})).unwrap()["total"], 11);

        let done = e
            .execute("row.update_many", &json!({"table": "orders", "ids": [1, 2, 3], "values": {"delivered": true}}))
            .unwrap();
        assert_eq!(done["updated"], 3);
        let done = e
            .execute(
                "row.update_many",
                &json!({"table": "orders", "where": {"customer.city": "Berlin"}, "values": {"quantity": 7}}),
            )
            .unwrap();
        assert_eq!(done["updated"], 3);

        // No rows named: an error, and nothing happens.
        for params in [
            json!({"table": "orders"}),
            json!({"table": "orders", "ids": []}),
            json!({"table": "orders", "ids": ["x"]}),
        ] {
            assert!(e.execute("row.delete_many", &params).is_err(), "{params}");
        }
        assert!(e.execute("row.update_many", &json!({"table": "orders", "values": {"quantity": 0}})).is_err());
        assert_eq!(e.execute("row.list", &json!({"table": "orders"})).unwrap()["total"], 11);

        let gone = e.execute("row.delete_many", &json!({"table": "orders", "ids": [10, 11, 99]})).unwrap();
        assert_eq!(gone["deleted"], 2);
        let gone = e.execute("row.delete_many", &json!({"table": "orders", "where": {"quantity": 7}})).unwrap();
        assert_eq!(gone["deleted"], 3);
        assert_eq!(e.execute("row.list", &json!({"table": "orders"})).unwrap()["total"], 6);
    }

    #[test]
    fn undo_and_backups_through_commands() {
        let (dir, e) = engine();
        e.execute("sample.load", &json!({"name": "orders"})).unwrap();
        assert!(dir.path().join("backups").is_dir(), "today's backup was made when the project opened");
        assert_eq!(e.execute("backup.daily", &json!({})).unwrap()["made"], false, "once a day");

        // Undo a bulk delete, then a single change, by default the newest first.
        e.execute("row.delete_many", &json!({"table": "customers", "where": {"city": "Lisbon"}})).unwrap();
        e.execute("row.update", &json!({"table": "orders", "id": 2, "values": {"item": "Changed"}})).unwrap();
        let list = e.execute("undo.list", &json!({})).unwrap();
        assert_eq!(list[0]["summary"], "Changed row 2 in orders");
        assert_eq!(list[1]["summary"], "Deleted 2 rows from customers");
        assert_eq!(e.execute("undo.run", &json!({})).unwrap()["reverted"], 1);
        assert_eq!(e.execute("undo.run", &json!({})).unwrap()["restored"], 2);
        let page = e.execute("row.list", &json!({"table": "orders", "where": {"customer.city": "Lisbon"}})).unwrap();
        assert_eq!(page["total"], 3, "customers and the links to them are back");
        assert!(e.execute("undo.run", &json!({})).is_err(), "nothing left to undo");

        // Back up now (with a function's code), wreck things, restore.
        let code = dir.path().join("functions/hello/main.ts");
        std::fs::create_dir_all(code.parent().unwrap()).unwrap();
        std::fs::write(&code, "// version 1").unwrap();
        let name = e.execute("backup.create", &json!({})).unwrap()["name"].as_str().unwrap().to_owned();
        std::fs::write(&code, "// version 2, a mistake").unwrap();
        e.execute("field.drop", &json!({"table": "orders", "name": "customer"})).unwrap();
        e.execute("table.drop", &json!({"name": "customers"})).unwrap();
        let points = e.execute("backup.list", &json!({})).unwrap();
        assert!(points.as_array().unwrap().iter().any(|p| p["label"] == "Backup made by hand"));
        let done = e.execute("backup.restore", &json!({"name": name})).unwrap();
        assert!(done["saved_as"].as_str().unwrap().ends_with("before-restore.db"));
        assert_eq!(e.execute("row.list", &json!({"table": "customers"})).unwrap()["total"], 5);
        assert_eq!(std::fs::read_to_string(&code).unwrap(), "// version 1", "the function's code is back too");
        // Restoring the "before" state brings the mistake back: a restore can be reversed.
        e.execute("backup.restore", &json!({"name": done["saved_as"]})).unwrap();
        assert_eq!(std::fs::read_to_string(&code).unwrap(), "// version 2, a mistake");
        assert!(e.execute("row.list", &json!({"table": "customers"})).is_err());
        assert!(e.execute("backup.restore", &json!({"name": "../../etc/passwd"})).is_err());
        assert!(e.execute("backup.restore", &json!({})).is_err());
    }

    #[test]
    fn stats_count_what_happens() {
        let (_dir, e) = engine();
        e.execute("sample.load", &json!({"name": "orders"})).unwrap();
        let secret =
            e.execute("api.key.create", &json!({"name": "shop site"})).unwrap()["secret"].as_str().unwrap().to_owned();
        let before = e.execute("stats.get", &json!({})).unwrap();
        assert_eq!(before["requests"]["total"], 0);
        assert_eq!((before["project"]["tables"].as_u64(), before["project"]["rows"].as_u64()), (Some(2), Some(14)));
        assert!(before["project"]["size_bytes"].as_u64().unwrap() > 0);
        assert_eq!(before["project"]["keys"], 1);
        assert_eq!(before["project"]["backups"], 1, "today's");

        let port = e.execute("api.start", &json!({})).unwrap()["port"].as_u64().unwrap() as u16;
        for _ in 0..3 {
            assert!(http_get(port, "/api/orders?limit=1", &secret).contains("\"total\""));
        }
        http_get(port, "/api/customers", &secret);
        http_get(port, "/api/orders", "pk_wrong");
        let s = e.execute("stats.get", &json!({})).unwrap();
        assert_eq!((s["requests"]["total"].as_u64(), s["requests"]["refused"].as_u64()), (Some(5), Some(1)));
        assert_eq!((s["keys"][0]["name"].as_str(), s["keys"][0]["requests"].as_u64()), (Some("shop site"), Some(4)));
        let orders = s["tables"].as_array().unwrap().iter().find(|t| t["name"] == "orders").unwrap();
        assert_eq!((orders["rows"].as_u64(), orders["requests"].as_u64()), (Some(9), Some(3)));
        assert_eq!(s["per_minute"].as_array().unwrap().iter().filter_map(Value::as_u64).sum::<u64>(), 5);
        // Turning the API off and on again doesn't lose the count.
        e.execute("api.stop", &json!({})).unwrap();
        assert_eq!(e.execute("stats.get", &json!({})).unwrap()["requests"]["total"], 5);
    }

    #[test]
    fn offline_engine_never_listens() {
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new_offline(Store::open(dir.path().join("p.db")).unwrap().into_shared());
        assert!(e.execute("api.start", &json!({})).is_err());
        assert_eq!(e.execute("api.status", &json!({})).unwrap()["running"], false);
    }

    fn http_get(port: u16, path: &str, key: &str) -> String {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {key}\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn command_ids_are_unique_and_documented() {
        let mut seen = std::collections::HashSet::new();
        for c in registry() {
            assert!(seen.insert(c.id), "duplicate command id {}", c.id);
            assert!(!c.label.is_empty() && !c.params.is_empty(), "{} needs a label and params doc", c.id);
            assert!(c.id.contains('.'), "{} should be namespaced", c.id);
        }
    }

    /// Fuzzes every command with empty and wrongly typed parameters. None may panic.
    #[test]
    fn every_command_survives_empty_params() {
        let hostile = [
            json!(null),
            json!({}),
            json!([]),
            json!("x"),
            json!(42),
            json!({"table": 5, "name": [], "id": "x", "values": 7, "fields": "x", "where": [], "limit": -1, "port": "x"}),
            json!({"table": "t", "name": "", "id": i64::MAX, "values": {}, "fields": [{}], "where": {"a": {}}, "port": 0}),
            json!({"table": "_porchly_keys", "name": "sqlite_master", "id": -1, "fields": [{"name": "id", "type": "blob"}], "port": 70000}),
            json!({"table": "t", "id": 1, "values": {"a": [1, 2]}, "where": {"a": null}, "sort": "-zzz", "offset": u64::MAX, "port": 80, "exact": true}),
        ];
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open(dir.path().join("p.db")).unwrap();
        store
            .create_table(
                &porchly_core::Ident::new("t").unwrap(),
                vec![porchly_schema::FieldDef {
                    name: porchly_core::Ident::new("a").unwrap(),
                    kind: porchly_core::FieldKind::Text,
                    required: false,
                    links_to: None,
                }],
            )
            .unwrap();
        let shared = store.into_shared();
        let net = Net::new(false);
        let functions = Arc::new(functions_service(&shared));
        let light = light_for(&shared, true);
        for c in registry() {
            for params in &hostile {
                let mut cx = Ctx { shared: &shared, guard: None, net: &net, functions: &functions, light: &light };
                let outcome = catch_unwind(AssertUnwindSafe(|| (c.run)(&mut cx, params)));
                assert!(outcome.is_ok(), "{} panicked with params {params}", c.id);
            }
        }
    }
}
