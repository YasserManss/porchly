//! A readable guide to a project's API, generated from its tables: how to authenticate,
//! every table's fields and routes with ready-to-run examples, conditions, links and errors.
//! Plain HTML and CSS, no scripts. Every name is escaped.

use porchly_core::FieldKind;
use porchly_schema::TableDef;

use crate::{FnAccess, FnSummary};

/// The guide as a complete HTML page. `base` is the API's address (`http://127.0.0.1:11911`).
pub fn html(tables: &[TableDef], functions: &[FnSummary], base: &str) -> String {
    let base = esc(base.trim_end_matches('/'));
    let mut toc = String::new();
    let mut sections = String::new();
    for t in tables {
        let name = esc(t.name.as_str());
        toc.push_str(&format!("<li><a href=\"#t-{name}\">{name}</a></li>"));
        sections.push_str(&table_section(t, &base));
    }
    if tables.is_empty() {
        sections.push_str("<p>This project has no tables yet. Make one in the app, and it appears here.</p>");
    }
    let functions = functions_section(functions, &base);
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Porchly API</title><style>{CSS}</style></head>
<body><main>
<h1>Your Porchly API</h1>
<p class="lead">Apps and websites use your tables through this API. Every address below starts with <code>{base}</code>.</p>

<h2 id="keys">Keys</h2>
<p>Every request needs a key, sent in the <code>Authorization</code> header. Make keys in the Porchly app under <b>Share</b>. A key is shown once; keys can only read unless you allowed them to make changes.</p>
<pre>curl -H "Authorization: Bearer YOUR_KEY" {base}/api</pre>

<h2>Tables</h2>
<ul class="toc">{toc}</ul>
{sections}
{functions}
<h2 id="lists">Lists: paging, sorting and conditions</h2>
<table><tr><th>Add to the address</th><th>What it does</th></tr>
<tr><td><code>?limit=20&amp;offset=40</code></td><td>Rows per page (up to 500) and rows to skip.</td></tr>
<tr><td><code>?sort=name</code>, <code>?sort=-name</code></td><td>Sort A→Z, or Z→A with a minus.</td></tr>
<tr><td><code>?search=ann</code></td><td>Rows with this text in any text field, ignoring case.</td></tr>
<tr><td><code>?field=value</code></td><td>Only rows where the field is exactly the value.</td></tr>
<tr><td><code>?field=op.value</code></td><td>Other conditions, listed below. All conditions must match.</td></tr>
<tr><td><code>?expand=link</code></td><td>Include the rows that link fields point to, under <code>"expand"</code>.</td></tr>
<tr><td><code>?link.field=value</code></td><td>Conditions and sorting on a linked table's fields, like <code>customer.city=Paris</code>.</td></tr>
</table>
<table><tr><th>Operator</th><th>Meaning</th><th>Example</th></tr>
<tr><td><code>ne</code></td><td>not equal</td><td><code>?status=ne.done</code></td></tr>
<tr><td><code>lt</code> <code>lte</code> <code>gt</code> <code>gte</code></td><td>less / greater than (or equal)</td><td><code>?price=lt.10</code></td></tr>
<tr><td><code>contains</code> <code>starts</code> <code>ends</code></td><td>text search, ignoring case</td><td><code>?name=contains.ann</code></td></tr>
<tr><td><code>in</code></td><td>one of a list</td><td><code>?size=in.S,M</code></td></tr>
<tr><td><code>is.empty</code> <code>is.notempty</code></td><td>has no value / has a value</td><td><code>?email=is.empty</code></td></tr>
</table>

<h2 id="errors">Errors</h2>
<p>Errors come back as <code>{{"error": {{"code": "...", "message": "..."}}}}</code>. The message is written for people.</p>
<table><tr><th>Status</th><th>Code</th><th>When</th></tr>
<tr><td>400</td><td><code>invalid</code></td><td>A value doesn't fit its field, or a condition is wrong.</td></tr>
<tr><td>401</td><td><code>unauthorized</code></td><td>No key, or a key that was turned off.</td></tr>
<tr><td>403</td><td><code>forbidden</code></td><td>The key can only read.</td></tr>
<tr><td>404</td><td><code>not_found</code></td><td>No such table or row.</td></tr>
<tr><td>413</td><td><code>too_large</code></td><td>The request is too big.</td></tr>
<tr><td>429</td><td><code>rate_limited</code></td><td>Too many requests with one key; slow down.</td></tr>
</table>
<p class="foot">Tools can read a machine-readable description at <code>{base}/api/openapi.json</code>.</p>
</main></body></html>"#
    )
}

fn functions_section(functions: &[FnSummary], base: &str) -> String {
    if functions.is_empty() {
        return String::new();
    }
    let mut rows = String::new();
    for f in functions {
        let name = esc(&f.name);
        let who = match f.access {
            FnAccess::Key => "Needs a key",
            FnAccess::Public => "<b>Anyone, no key</b>",
        };
        rows.push_str(&format!(
            "<tr><td><code>{name}</code></td><td>{who}</td><td><code>{base}/api/fn/{name}</code></td></tr>"
        ));
    }
    let first = functions.first().map(|f| esc(&f.name)).unwrap_or_default();
    format!(
        r#"<h2 id="functions">Functions</h2>
<p>Small programs the owner added. Each one decides what it accepts and returns. Addresses below a function's address reach it too, like <code>{base}/api/fn/{first}/orders/7</code>.</p>
<table><tr><th>Function</th><th>Who can run it</th><th>Address</th></tr>{rows}</table>
<pre>curl -X POST -H "Authorization: Bearer YOUR_KEY" \
  -d '{{"hello": "world"}}' {base}/api/fn/{first}</pre>
"#
    )
}

fn table_section(t: &TableDef, base: &str) -> String {
    let name = esc(t.name.as_str());
    let mut rows = String::from(
        "<tr><td><code>id</code></td><td>Whole number</td><td></td><td>Set by Porchly</td></tr>\
         <tr><td><code>created_at</code>, <code>updated_at</code></td><td>Date and time</td><td></td><td>Set by Porchly</td></tr>",
    );
    for f in &t.fields {
        let note = match &f.links_to {
            Some(target) => format!("The id of a row in <a href=\"#t-{0}\">{0}</a>", esc(target.as_str())),
            None => String::new(),
        };
        rows.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{note}</td></tr>",
            esc(f.name.as_str()),
            esc(f.kind.label()),
            if f.required { "Required" } else { "" },
        ));
    }
    let example = example_body(t);
    let first = t.fields.first().map(|f| esc(f.name.as_str())).unwrap_or_else(|| "field".into());
    format!(
        r#"<section id="t-{name}"><h3>{name}</h3>
<table><tr><th>Field</th><th>Type</th><th></th><th>Notes</th></tr>{rows}</table>
<h4>List rows</h4><pre>curl -H "Authorization: Bearer YOUR_KEY" "{base}/api/{name}?limit=20&amp;sort=-{first}"</pre>
<h4>Get one row</h4><pre>curl -H "Authorization: Bearer YOUR_KEY" {base}/api/{name}/1</pre>
<h4>Add a row</h4><p>Needs a key that can make changes.</p>
<pre>curl -X POST -H "Authorization: Bearer YOUR_KEY" \
  -d '{example}' {base}/api/{name}</pre>
<h4>Change a row</h4><p>Send only the fields to change.</p>
<pre>curl -X PATCH -H "Authorization: Bearer YOUR_KEY" \
  -d '{example}' {base}/api/{name}/1</pre>
<h4>Delete a row</h4><pre>curl -X DELETE -H "Authorization: Bearer YOUR_KEY" {base}/api/{name}/1</pre>
</section>"#
    )
}

/// A JSON body with an example value for each field.
fn example_body(t: &TableDef) -> String {
    let parts: Vec<String> = t
        .fields
        .iter()
        .take(6)
        .map(|f| {
            let value = match f.kind {
                FieldKind::Text => "\"some text\"",
                FieldKind::Integer => "3",
                FieldKind::Number => "4.5",
                FieldKind::Boolean => "true",
                FieldKind::Datetime => "\"2026-01-15\"",
                FieldKind::Link => "1",
            };
            format!("\"{}\": {value}", f.name.as_str())
        })
        .collect();
    esc(&format!("{{{}}}", parts.join(", ")))
}

/// Escapes text for HTML.
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

const CSS: &str = "
:root { --bg:#faf8f5; --fg:#24211e; --dim:#6b655e; --line:#e2dcd2; --card:#fff; --accent:#b06507; --code:#f2eee8; }
@media (prefers-color-scheme: dark) { :root { --bg:#1c1a18; --fg:#eeeae4; --dim:#a8a096; --line:#3d3833; --card:#25221f; --accent:#f2a73b; --code:#2b2824; } }
body { background:var(--bg); color:var(--fg); font:16px/1.55 system-ui, sans-serif; margin:0; }
main { max-width:860px; margin:0 auto; padding:32px 20px 64px; }
h1 { font-size:30px; margin:0 0 8px; } h2 { margin-top:40px; border-bottom:1px solid var(--line); padding-bottom:6px; }
h3 { font-size:22px; color:var(--accent); margin-top:32px; } h4 { margin:18px 0 6px; }
.lead, .foot { color:var(--dim); } a { color:var(--accent); }
code, pre { font-family:ui-monospace, SFMono-Regular, Menlo, monospace; font-size:14px; background:var(--code); border-radius:6px; }
code { padding:1px 5px; } pre { padding:12px 14px; overflow-x:auto; white-space:pre-wrap; word-break:break-all; }
table { border-collapse:collapse; width:100%; margin:10px 0; background:var(--card); }
th, td { text-align:left; padding:7px 10px; border-bottom:1px solid var(--line); vertical-align:top; }
th { color:var(--dim); font-weight:600; font-size:14px; }
section { background:var(--card); border:1px solid var(--line); border-radius:12px; padding:4px 20px 16px; margin:18px 0; }
.toc { columns:2; }
";

#[cfg(test)]
mod tests {
    use super::*;
    use porchly_core::Ident;
    use porchly_schema::FieldDef;

    #[test]
    fn describes_every_table_and_field() {
        let customers = TableDef { name: Ident::new("customers").unwrap(), fields: vec![] };
        let orders = TableDef {
            name: Ident::new("orders").unwrap(),
            fields: vec![
                FieldDef::new(Ident::new("item").unwrap(), FieldKind::Text, true),
                FieldDef::link(Ident::new("customer").unwrap(), Ident::new("customers").unwrap(), false),
            ],
        };
        let page = html(&[customers, orders], &[], "http://127.0.0.1:11911/");
        assert!(page.contains("id=\"t-orders\""));
        assert!(page.contains("<code>item</code></td><td>Text</td><td>Required"));
        assert!(page.contains("The id of a row in <a href=\"#t-customers\">customers</a>"));
        assert!(page.contains("http://127.0.0.1:11911/api/orders/1"), "no double slash");
        assert!(!page.contains("<script"));
        assert!(!page.contains("id=\"functions\""), "no functions, no section");
    }

    #[test]
    fn lists_functions() {
        let fns = [
            FnSummary { name: "send_invoice".into(), access: FnAccess::Key },
            FnSummary { name: "stripe_hook".into(), access: FnAccess::Public },
        ];
        let page = html(&[], &fns, "http://127.0.0.1:11911");
        assert!(page.contains("<code>http://127.0.0.1:11911/api/fn/send_invoice</code>"));
        assert!(page.contains("<td><code>stripe_hook</code></td><td><b>Anyone, no key</b>"));
    }

    #[test]
    fn escapes_text() {
        assert_eq!(esc("<a href='x'>&\"</a>"), "&lt;a href=&#39;x&#39;&gt;&amp;&quot;&lt;/a&gt;");
        let page = html(
            &[],
            &[FnSummary { name: "<b>".into(), access: FnAccess::Key }],
            "http://x\"><script>alert(1)</script>",
        );
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("<code>&lt;b&gt;</code>"));
    }
}
