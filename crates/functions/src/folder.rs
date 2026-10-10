//! The `functions/` folder next to the project: one folder per function with its code
//! (`main.ts`, or `main.js`) and Porchly's settings for it (`function.json`). Real files, so
//! people can also edit them in any editor; saving is enough, nothing needs restarting.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use porchly_api::FnAccess;
use porchly_core::{Error, Ident, Result, limits};
use serde::{Deserialize, Serialize};

const SETTINGS_FILE: &str = "function.json";
const ENTRY_FILES: [&str; 2] = ["main.ts", "main.js"];
/// Deleted functions are moved here, not erased.
const DELETED_DIR: &str = ".deleted";

/// Porchly's settings for one function (`function.json`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub access: FnAccess,
    /// Variables the function reads with `process.env` (for example another service's key).
    pub env: BTreeMap<String, String>,
}

/// A function as listed.
#[derive(Debug, Clone, Serialize)]
pub struct FnInfo {
    pub name: String,
    pub access: FnAccess,
    /// The code file, relative to the functions folder (`hello/main.ts`).
    pub file: String,
}

/// Starting points for new functions.
pub const TEMPLATES: [&str; 2] = ["hello", "tables"];

/// The functions folder of a project.
#[derive(Debug, Clone)]
pub struct Folder {
    root: PathBuf,
}

impl Folder {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Folder { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every function, by name. Folders without code, and names Porchly can't serve, are skipped.
    pub fn list(&self) -> Vec<FnInfo> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut out: Vec<FnInfo> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_str()?.to_owned();
                Ident::new(&name).ok()?;
                let file = self.entry(&name)?;
                let settings = self.settings(&name).unwrap_or_default();
                let file = format!("{name}/{}", file.file_name()?.to_str()?);
                Some(FnInfo { name, access: settings.access, file })
            })
            .take(limits::MAX_FUNCTIONS)
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// The code file of `name`, if it has one.
    pub fn entry(&self, name: &str) -> Option<PathBuf> {
        let dir = self.root.join(name);
        ENTRY_FILES.iter().map(|f| dir.join(f)).find(|p| p.is_file())
    }

    fn dir_of(&self, name: &str) -> Result<PathBuf> {
        let name = valid_name(name)?;
        let dir = self.root.join(name.as_str());
        if self.entry(name.as_str()).is_none() {
            return Err(Error::not_found(format!("There's no function \"{name}\".")));
        }
        Ok(dir)
    }

    pub fn exists(&self, name: &str) -> bool {
        Ident::new(name).is_ok() && self.entry(name).is_some()
    }

    /// A function's settings. A missing or unreadable `function.json` means the defaults
    /// (needs a key), so a broken file never makes a function public.
    pub fn settings(&self, name: &str) -> Result<Settings> {
        let dir = self.dir_of(name)?;
        let Ok(text) = std::fs::read_to_string(dir.join(SETTINGS_FILE)) else { return Ok(Settings::default()) };
        Ok(serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!("functions/{name}/{SETTINGS_FILE} isn't valid ({e}); using the defaults (needs a key)");
            Settings::default()
        }))
    }

    pub fn set_settings(&self, name: &str, settings: &Settings) -> Result<()> {
        if settings.env.len() > limits::MAX_FUNCTION_VARS {
            return Err(Error::invalid(format!(
                "A function can have at most {} variables.",
                limits::MAX_FUNCTION_VARS
            )));
        }
        for (k, v) in &settings.env {
            check_var(k, v)?;
        }
        let dir = self.dir_of(name)?;
        let text = serde_json::to_string_pretty(settings).map_err(|e| Error::Internal(e.to_string()))?;
        write_atomic(&dir.join(SETTINGS_FILE), text.as_bytes())
    }

    /// Makes a new function from a template (`hello` when `None`).
    pub fn create(&self, name: &str, template: Option<&str>) -> Result<FnInfo> {
        let ident = valid_name(name)?;
        let dir = self.root.join(ident.as_str());
        if dir.exists() {
            return Err(Error::Conflict(format!("There's already a function called \"{ident}\".")));
        }
        if self.list().len() >= limits::MAX_FUNCTIONS {
            return Err(Error::invalid(format!("A project can have at most {} functions.", limits::MAX_FUNCTIONS)));
        }
        let code = template_code(template.unwrap_or("hello"), ident.as_str())?;
        std::fs::create_dir_all(&dir).map_err(|e| io_error(&dir, &e))?;
        write_atomic(&dir.join("main.ts"), code.as_bytes())?;
        let settings =
            serde_json::to_string_pretty(&Settings::default()).map_err(|e| Error::Internal(e.to_string()))?;
        write_atomic(&dir.join(SETTINGS_FILE), settings.as_bytes())?;
        Ok(FnInfo { name: ident.as_str().to_owned(), access: FnAccess::Key, file: format!("{ident}/main.ts") })
    }

    pub fn code(&self, name: &str) -> Result<String> {
        self.dir_of(name)?;
        let file = self.entry(name).ok_or_else(|| Error::not_found(format!("There's no function \"{name}\".")))?;
        std::fs::read_to_string(&file).map_err(|e| io_error(&file, &e))
    }

    pub fn save(&self, name: &str, code: &str) -> Result<()> {
        if code.len() > limits::MAX_FUNCTION_CODE_BYTES {
            return Err(Error::TooLarge(format!(
                "The code is too long (the limit is {} KB). Split it into several files in the function's folder.",
                limits::MAX_FUNCTION_CODE_BYTES / 1024
            )));
        }
        self.dir_of(name)?;
        let file = self.entry(name).ok_or_else(|| Error::not_found(format!("There's no function \"{name}\".")))?;
        write_atomic(&file, code.as_bytes())
    }

    pub fn rename(&self, name: &str, new_name: &str) -> Result<FnInfo> {
        let from = self.dir_of(name)?;
        let to_name = valid_name(new_name)?;
        let to = self.root.join(to_name.as_str());
        if to.exists() {
            return Err(Error::Conflict(format!("There's already a function called \"{to_name}\".")));
        }
        std::fs::rename(&from, &to).map_err(|e| io_error(&to, &e))?;
        self.list()
            .into_iter()
            .find(|f| f.name == to_name.as_str())
            .ok_or_else(|| Error::Internal("renamed function vanished".into()))
    }

    /// Moves a function into `functions/.deleted/`, so its code can still be found.
    pub fn delete(&self, name: &str) -> Result<PathBuf> {
        let from = self.dir_of(name)?;
        let bin = self.root.join(DELETED_DIR);
        std::fs::create_dir_all(&bin).map_err(|e| io_error(&bin, &e))?;
        let stamp =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
        let to = bin.join(format!("{name}-{stamp}"));
        std::fs::rename(&from, &to).map_err(|e| io_error(&to, &e))?;
        Ok(to)
    }
}

fn valid_name(name: &str) -> Result<Ident> {
    Ident::new(name).map_err(|_| {
        Error::invalid(format!(
            "\"{}\" can't be a function name. Use lowercase letters, digits and underscores, starting with a letter (for example \"send_invoice\").",
            name.chars().take(40).collect::<String>()
        ))
    })
}

fn check_var(name: &str, value: &str) -> Result<()> {
    let ok_name = !name.is_empty()
        && name.len() <= 100
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok_name {
        return Err(Error::invalid(format!(
            "\"{}\" can't be a variable name. Use letters, digits and underscores, like MAIL_KEY.",
            name.chars().take(40).collect::<String>()
        )));
    }
    if name.to_ascii_uppercase().starts_with("PORCHLY_") {
        return Err(Error::invalid("Variable names starting with PORCHLY_ are reserved for Porchly."));
    }
    if value.len() > 10_000 || value.contains('\0') {
        return Err(Error::invalid(format!("The value of {name} is too long or has invalid characters.")));
    }
    Ok(())
}

/// Writes a file completely or not at all.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let part = path.with_extension("part");
    {
        let mut f = std::fs::File::create(&part).map_err(|e| io_error(&part, &e))?;
        f.write_all(bytes).map_err(|e| io_error(&part, &e))?;
        f.flush().map_err(|e| io_error(&part, &e))?;
    }
    std::fs::rename(&part, path).map_err(|e| io_error(path, &e))
}

fn io_error(path: &Path, e: &std::io::Error) -> Error {
    Error::invalid(format!("Couldn't use \"{}\": {e}", path.display()))
}

fn template_code(template: &str, name: &str) -> Result<String> {
    let code = match template {
        "hello" => HELLO,
        "tables" => TABLES,
        other => {
            return Err(Error::invalid(format!(
                "There's no template \"{}\". Templates: {}.",
                other.chars().take(40).collect::<String>(),
                TEMPLATES.join(", ")
            )));
        }
    };
    Ok(code.replace("{name}", name))
}

const HELLO: &str = r#"// Porchly runs this function when someone calls /api/fn/{name}
// It gets a standard web Request and returns a Response, like Cloudflare Workers and Bun.
// Learn more: https://developer.mozilla.org/en-US/docs/Web/API/Request

export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const name = url.searchParams.get("name") ?? "world";
    return Response.json({ hello: name, path: url.pathname, method: request.method });
  },
};
"#;

const TABLES: &str = r#"// Porchly runs this function when someone calls /api/fn/{name}
// It reads and changes your tables through Porchly's own API, with a key that only
// functions get (PORCHLY_URL and PORCHLY_TOKEN are set by Porchly).

const api = process.env.PORCHLY_URL;
const headers = { Authorization: `Bearer ${process.env.PORCHLY_TOKEN}`, "Content-Type": "application/json" };

export default {
  async fetch(request: Request): Promise<Response> {
    // Which tables does this project have?
    const res = await fetch(`${api}/api`, { headers });
    const { tables } = await res.json();
    const names = tables.map((t: { name: string }) => t.name);

    // Read the first rows of the first table, if there is one.
    let rows = [];
    if (names.length > 0) {
      const page = await (await fetch(`${api}/api/${names[0]}?limit=5`, { headers })).json();
      rows = page.items;
    }
    // To add a row:   await fetch(`${api}/api/recipes`, { method: "POST", headers, body: JSON.stringify({ name: "Soup" }) });
    // To change one:  await fetch(`${api}/api/recipes/1`, { method: "PATCH", headers, body: JSON.stringify({ name: "Stew" }) });
    return Response.json({ tables: names, first_rows: rows });
  },
};
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn folder() -> (tempfile::TempDir, Folder) {
        let dir = tempfile::tempdir().unwrap();
        let f = Folder::new(dir.path().join("functions"));
        (dir, f)
    }

    #[test]
    fn create_edit_rename_delete() {
        let (_dir, f) = folder();
        assert!(f.list().is_empty(), "no folder yet is fine");
        let info = f.create("hello", None).unwrap();
        assert_eq!((info.name.as_str(), info.file.as_str(), info.access), ("hello", "hello/main.ts", FnAccess::Key));
        assert!(f.code("hello").unwrap().contains("/api/fn/hello"));
        assert!(f.create("hello", None).is_err(), "no duplicates");
        assert!(f.create("Bad-Name", None).is_err());
        assert!(f.create("x", Some("nope")).is_err());

        f.save("hello", "export default { fetch: () => new Response('hi') };").unwrap();
        assert!(f.code("hello").unwrap().contains("'hi'"));
        assert!(f.save("hello", &"x".repeat(limits::MAX_FUNCTION_CODE_BYTES + 1)).is_err());

        f.set_settings(
            "hello",
            &Settings { access: FnAccess::Public, env: [("MAIL_KEY".into(), "s3cret".into())].into() },
        )
        .unwrap();
        assert_eq!(f.list()[0].access, FnAccess::Public);
        assert!(
            f.set_settings(
                "hello",
                &Settings { env: [("PORCHLY_TOKEN".into(), "x".into())].into(), ..Settings::default() }
            )
            .is_err()
        );
        assert!(
            f.set_settings("hello", &Settings { env: [("bad name".into(), "x".into())].into(), ..Settings::default() })
                .is_err()
        );

        let renamed = f.rename("hello", "greet").unwrap();
        assert_eq!(renamed.access, FnAccess::Public, "settings travel with the function");
        assert!(f.code("hello").is_err());
        f.create("other", Some("tables")).unwrap();
        assert!(f.rename("greet", "other").is_err());

        let kept = f.delete("greet").unwrap();
        assert!(kept.join("main.ts").is_file(), "deleted code is kept");
        assert_eq!(f.list().iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["other"]);
    }

    #[test]
    fn hostile_names_never_escape_the_folder() {
        let (_dir, f) = folder();
        for name in ["../x", "..", "a/b", "a\\b", "", ".deleted", "C:", "x\0y"] {
            assert!(f.create(name, None).is_err(), "{name:?}");
            assert!(f.code(name).is_err(), "{name:?}");
            assert!(!f.exists(name));
        }
    }

    #[test]
    fn a_broken_settings_file_means_needs_a_key() {
        let (_dir, f) = folder();
        f.create("hook", None).unwrap();
        std::fs::write(f.root().join("hook").join(SETTINGS_FILE), "{ not json").unwrap();
        assert_eq!(f.settings("hook").unwrap().access, FnAccess::Key);
        std::fs::remove_file(f.root().join("hook").join(SETTINGS_FILE)).unwrap();
        assert_eq!(f.settings("hook").unwrap(), Settings::default());
    }

    #[test]
    fn plain_javascript_works_too() {
        let (_dir, f) = folder();
        std::fs::create_dir_all(f.root().join("js")).unwrap();
        std::fs::write(f.root().join("js").join("main.js"), "export default {}").unwrap();
        std::fs::create_dir_all(f.root().join("empty")).unwrap();
        assert_eq!(f.list().iter().map(|i| i.file.as_str()).collect::<Vec<_>>(), ["js/main.js"]);
    }
}
