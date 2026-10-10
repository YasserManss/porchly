//! The command registry. Adding a user-visible feature means adding a command here, with tests.

mod csv;
mod function;
mod key;
mod project;
mod row;
mod sample;
mod server;
mod settings;
mod table;
mod tunnel;

use porchly_core::Result;
use porchly_store::Store;
use serde_json::Value;

use crate::Ctx;
pub(crate) use function::{FUNCTIONS_SETTING, api_options};
pub(crate) use settings::{apply_settings, port_setting};
pub(crate) use tunnel::PORCH_LIGHT_CHANGES;

/// One user-visible action.
pub struct Command {
    /// Stable id, namespaced by area (`table.create`).
    pub id: &'static str,
    /// What people see in menus and buttons.
    pub label: &'static str,
    /// Where it appears in the app's menus (`Table/New table…`), or empty if it doesn't.
    pub menu: &'static str,
    pub shortcut: Option<&'static str>,
    /// Parameters, documented for people, scripts and agents.
    pub params: &'static str,
    /// Whether the command can run now.
    pub enabled: fn(&Store) -> bool,
    pub run: fn(&mut Ctx<'_>, &Value) -> Result<Value>,
}

pub(crate) fn always(_: &Store) -> bool {
    true
}

static REGISTRY: std::sync::LazyLock<Vec<Command>> = std::sync::LazyLock::new(|| {
    let mut all = Vec::new();
    all.extend(project::commands());
    all.extend(table::commands());
    all.extend(row::commands());
    all.extend(key::commands());
    all.extend(csv::commands());
    all.extend(server::commands());
    all.extend(tunnel::commands());
    all.extend(settings::commands());
    all.extend(function::commands());
    all.extend(sample::commands());
    all
});

/// Every command, in menu order.
pub fn registry() -> &'static [Command] {
    &REGISTRY
}

pub(crate) fn find(id: &str) -> Option<&'static Command> {
    registry().iter().find(|c| c.id == id)
}

pub(crate) fn to_json(v: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| porchly_core::Error::Internal(e.to_string()))
}
