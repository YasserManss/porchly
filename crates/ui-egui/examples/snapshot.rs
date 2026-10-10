//! Renders the app offscreen to a PNG, so changes can be looked at without opening a window.
//!
//! cargo run -p porchly-ui-egui --example snapshot -- out.png [--empty] [--share] [--dark]
//!     [--fresh-key] [--dialog new_table|add_field|add_link|edit_field|new_key|delete_row] [--import file.csv]
//!     [--sample recipes|orders] [--table name] [--size 1200x760] [--scale 2]
//!     [--functions off|on|public] (with PORCHLY_TEST_BUN set, "on" also tries the function)
//!     [--dialog functions|functions_progress|new_function|turn_off_functions|porch_light|settings]
//!     [--porch-light] (with PORCHLY_TEST_CLOUDFLARED set: really turns it on, needs the internet)

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui_kittest::kittest::Queryable;
use porchly_engine::Engine;
use porchly_store::Store;
use porchly_ui_egui::{Dialog, PorchlyApp};
use serde_json::json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let out = args.iter().find(|a| a.ends_with(".png")).cloned().unwrap_or_else(|| "porchly.png".into());
    let (w, h) = value("--size")
        .and_then(|s| s.split_once('x').map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap())))
        .unwrap_or((1200.0, 760.0));

    // Functions need a project on disk.
    let functions = value("--functions");
    let live_light = std::env::var_os("PORCHLY_TEST_CLOUDFLARED").filter(|_| has("--porch-light"));
    let live_bun = std::env::var_os("PORCHLY_TEST_BUN").filter(|_| functions.is_some());
    let dir = tempfile::tempdir().unwrap();
    let store = if functions.is_some() || live_light.is_some() {
        let mut s = Store::open(dir.path().join("porchly.db")).unwrap();
        if functions.as_deref() != Some("off") {
            s.set_setting("functions", Some("on")).unwrap();
        }
        s
    } else {
        Store::open_in_memory().unwrap()
    };
    let engine = if live_bun.is_some() || live_light.is_some() {
        Engine::new(store.into_shared())
    } else {
        Engine::new_offline(store.into_shared())
    };
    if let Some(cf) = &live_light {
        engine.use_cloudflared(Some(cf.into()));
    }
    if let Some(bun) = &live_bun {
        engine.use_bun(Some(bun.into()));
        engine.execute("api.start", &json!({"port": 11990})).unwrap();
    }
    if matches!(functions.as_deref(), Some("on" | "public")) {
        engine.execute("function.create", &json!({"name": "send_invoice", "template": "tables"})).unwrap();
        engine.execute("function.create", &json!({"name": "hello"})).unwrap();
        engine.execute("function.create", &json!({"name": "stripe_hook"})).unwrap();
        engine.execute("function.settings", &json!({"name": "stripe_hook", "access": "public"})).unwrap();
    }
    if let Some(sample) = value("--sample") {
        engine.execute("sample.load", &json!({"name": sample})).unwrap();
        engine.execute("api.key.create", &json!({"name": "Shop website"})).unwrap();
        engine.execute("api.key.create", &json!({"name": "Order app", "write": true})).unwrap();
    } else if !has("--empty") {
        engine.execute("sample.load", &json!({})).unwrap();
        engine.execute("api.key.create", &json!({"name": "My website"})).unwrap();
        engine.execute("api.key.create", &json!({"name": "Kitchen tablet", "write": true})).unwrap();
    }
    if let Some(n) = value("--many").and_then(|n| n.parse::<usize>().ok()) {
        engine.execute("table.create", &json!({"name": "visitors", "fields": [{"name": "name", "type": "text"}, {"name": "visits", "type": "integer"}]})).unwrap();
        for i in 1..=n {
            engine
                .execute(
                    "row.insert",
                    &json!({"table": "visitors", "values": {"name": format!("Visitor {i}"), "visits": i % 7}}),
                )
                .unwrap();
        }
    }
    let mut app = PorchlyApp::new(engine, "porchly.db");
    match functions.as_deref() {
        Some("on") => app.open_function("hello"),
        Some("public") => app.open_function("stripe_hook"),
        _ => {}
    }
    if live_light.is_some() {
        app.start_porch_light(false);
    }
    if live_bun.is_some() && functions.as_deref() == Some("on") {
        app.refresh_status_now();
        app.try_function();
    }
    if let Some(table) = value("--table") {
        app.select_table(&table);
    }
    if let Some(file) = value("--import") {
        app.begin_import(std::path::Path::new(&file));
    }
    if let Some(text) = value("--search") {
        app.set_search(&text);
    }
    app.show_share = has("--share");
    if has("--fresh-key") {
        app.show_new_key("Phone app", "pk_7x2kq9mf4wz8c3hj5ndr6tlpbv0aesg1yu4oiw7qkzm2xc9bnhta");
    }
    match value("--dialog").as_deref() {
        Some("new_table") => app.open_dialog(Dialog::NewTable { label: "Guest list".into(), with_name_field: true }),
        Some("add_field") => app.open_dialog(Dialog::add_field("recipes")),
        Some("add_link") => app.open_dialog(Dialog::AddField {
            table: "orders".into(),
            label: "Sold by".into(),
            kind: porchly_core::FieldKind::Link,
            required: false,
            links_to: Some("customers".into()),
        }),
        Some("edit_field") => {
            let mut d = Dialog::edit_field("recipes", "name", porchly_core::FieldKind::Text, true, None);
            if let Dialog::EditField { kind, .. } = &mut d {
                *kind = porchly_core::FieldKind::Integer;
            }
            app.open_dialog(d);
            if has("--confirm") {
                app.confirm_dialog();
            }
        }
        Some("new_key") => app.open_dialog(Dialog::NewKey { name: "My website".into(), can_write: false }),
        Some("delete_row") => app.open_dialog(Dialog::DeleteRow { table: "recipes".into(), id: 3 }),
        Some("functions") => app.open_dialog(Dialog::functions_opt_in()),
        Some("functions_progress") => {
            app.open_dialog(Dialog::FunctionsOptIn { progress: Some((14_200_000, 36_646_985)) })
        }
        Some("new_function") => {
            let mut d = Dialog::new_function();
            if let Dialog::NewFunction { label, .. } = &mut d {
                *label = "Send invoice".into();
            }
            app.open_dialog(d);
        }
        Some("settings") => app.open_settings(),
        Some("porch_light") => app.open_dialog(Dialog::PorchLight { allow_changes: false, public_functions: false }),
        Some("turn_off_functions") => app.open_dialog(Dialog::TurnOffFunctions { remove_runtime: false }),
        Some(other) => panic!("unknown dialog {other}"),
        None => {}
    }
    let dark = has("--dark");

    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(w, h))
        .with_pixels_per_point(value("--scale").and_then(|s| s.parse().ok()).unwrap_or(1.0))
        .with_max_steps(12)
        .wgpu()
        .build_ui_state(
            move |ui, app: &mut PorchlyApp| {
                PorchlyApp::setup(ui.ctx());
                ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                app.ui(ui);
            },
            app,
        );
    harness.run_steps(6);
    if live_light.is_some() {
        // Wait for the link (a real tunnel: up to a minute).
        for _ in 0..240 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            harness.run_steps(1);
            if harness.query_by_label("Allow changes through the link").is_some() {
                break;
            }
        }
        harness.run_steps(4);
    }
    if live_bun.is_some() {
        // Let the background call finish.
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(50));
            harness.run_steps(2);
        }
    }
    let img = harness.render().expect("render");
    img.save(&out).expect("save png");
    eprintln!("wrote {out} ({}x{})", img.width(), img.height());
}
