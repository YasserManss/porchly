//! Click-through tests of the desktop interface, driven like a person would (by button labels).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use porchly_engine::Engine;
use porchly_store::Store;
use porchly_ui_egui::PorchlyApp;
use serde_json::json;

fn harness(sample: bool) -> Harness<'static, PorchlyApp> {
    let engine = Engine::new_offline(Store::open_in_memory().unwrap().into_shared());
    if sample {
        engine.execute("sample.load", &json!({})).unwrap();
    }
    let app = PorchlyApp::new(engine, "test.db");
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui_state(
        |ui, app: &mut PorchlyApp| {
            PorchlyApp::setup(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

fn settle(h: &mut Harness<'static, PorchlyApp>) {
    h.run_steps(4);
}

#[test]
fn welcome_offers_a_sample_table() {
    let mut h = harness(false);
    assert!(h.query_by_label("Make your first table").is_some());
    h.get_by_label("Add a sample table").click();
    settle(&mut h);
    assert!(h.query_by_label("Make your first table").is_none());
    assert!(h.query_by_label("6 rows").is_some());
}

#[test]
fn making_a_table_from_a_friendly_name() {
    let mut h = harness(false);
    h.get_by_label("Make your first table").click();
    settle(&mut h);
    assert!(h.query_by_label("Apps will reach it at /api/guest_list").is_none());
    h.get_by_role(egui::accesskit::Role::TextInput).type_text("Guest List");
    settle(&mut h);
    assert!(h.query_by_label("Apps will reach it at /api/guest_list").is_some());
    h.get_by_label("Make table").click();
    settle(&mut h);
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Guest list").is_some(), "the table is listed");
    assert_eq!(h.query_all_by_label("Guest list").count(), 2, "listed in the sidebar and shown as the heading");
    assert!(h.query_by_label("0 rows").is_some());
}

#[test]
fn a_new_key_is_shown_once_with_a_copy_button() {
    let mut h = harness(true);
    h.get_by_label("Share").click();
    settle(&mut h);
    assert!(h.query_by_label("No keys yet, so no app can use your tables.").is_some());
    h.get_by_label("New key").click();
    settle(&mut h);
    // The grid's cells are text boxes too; the dialog's field is the focused one.
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.accesskit_node().is_focused())
        .expect("the dialog focuses its first field")
        .type_text("My website");
    settle(&mut h);
    h.get_by_label("Make key").click();
    settle(&mut h);
    assert!(h.query_by_label("Copy key").is_some());
    assert!(h.query_by_label("My website").is_some());
    h.get_by_label("Done").click();
    settle(&mut h);
    assert!(h.query_by_label("Copy key").is_none(), "the secret is gone once dismissed");
}

#[test]
fn deleting_a_row_asks_first() {
    let mut h = harness(true);
    h.get_all_by_label("Delete this row…").next().unwrap().click();
    settle(&mut h);
    assert!(h.query_by_label("Delete row 1?").is_some());
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(h.query_by_label("6 rows").is_some(), "cancel keeps the row");
    h.get_all_by_label("Delete this row…").next().unwrap().click();
    settle(&mut h);
    h.get_by_label("Delete row").click();
    settle(&mut h);
    assert!(h.query_by_label("5 rows").is_some());
}

#[test]
fn offline_api_reports_itself_off() {
    let mut h = harness(true);
    h.get_by_label("Share").click();
    settle(&mut h);
    h.get_by_label("Turn on").click();
    settle(&mut h);
    assert!(h.query_by_label("API off").is_some());
    assert!(h.query_by_label("The API can't be turned on in this mode.").is_some(), "the reason is shown");
}

#[test]
fn rows_added_elsewhere_appear_on_their_own() {
    let engine = Engine::new_offline(Store::open_in_memory().unwrap().into_shared());
    engine.execute("sample.load", &json!({})).unwrap();
    let outside = engine.clone();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui_state(
        |ui, app: &mut PorchlyApp| {
            PorchlyApp::setup(ui.ctx());
            app.ui(ui);
        },
        PorchlyApp::new(engine, "test.db"),
    );
    h.run_steps(3);
    assert!(h.query_by_label("6 rows").is_some());
    // An app adds a row through the API (here: straight through the engine).
    outside.execute("row.insert", &json!({"table": "recipes", "values": {"name": "Soda bread"}})).unwrap();
    h.step();
    let start = std::time::Instant::now();
    while h.query_by_label("7 rows").is_none() && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        h.step();
    }
    assert!(h.query_by_label("7 rows").is_some());
}

/// Presses the mouse at `from`, moves to `to` and releases.
fn drag(h: &mut Harness<'static, PorchlyApp>, from: egui::Pos2, to: egui::Pos2) {
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    let mid = from + (to - from) / 2.0;
    for e in [
        egui::Event::PointerMoved(from),
        button(from, true),
        egui::Event::PointerMoved(mid),
        egui::Event::PointerMoved(to),
        button(to, false),
    ] {
        h.event(e);
        h.step();
    }
    settle(h);
}

fn left_of(h: &Harness<'static, PorchlyApp>, label: &str) -> f64 {
    h.get_by_label(label).accesskit_node().bounding_box().map(|b| b.x0).unwrap()
}

#[test]
fn the_share_panel_can_be_resized_by_dragging_its_edge() {
    let mut h = harness(true);
    h.get_by_label("Share").click();
    settle(&mut h);
    let before = left_of(&h, "Your API");
    let edge_x = 1200.0 - 360.0;
    drag(&mut h, egui::pos2(edge_x, 400.0), egui::pos2(edge_x - 200.0, 400.0));
    let after = left_of(&h, "Your API");
    assert!(before - after > 150.0, "panel content moved from x={before} to x={after}");
}

#[test]
fn the_tables_sidebar_can_be_resized_by_dragging_its_edge() {
    let mut h = harness(true);
    let before = left_of(&h, "6 rows");
    drag(&mut h, egui::pos2(220.0, 400.0), egui::pos2(370.0, 400.0));
    let after = left_of(&h, "6 rows");
    assert!(after - before > 100.0, "the table moved from x={before} to x={after}");
}

#[test]
fn tables_in_the_sidebar_are_labelled_for_screen_readers() {
    let h = harness(true);
    let node = h.get_by_role_and_label(egui::accesskit::Role::Button, "Recipes");
    assert_eq!(node.accesskit_node().toggled(), Some(egui::accesskit::Toggled::True), "the open table is marked");
}

#[test]
fn a_link_is_changed_by_picking_a_row_by_name() {
    let engine = Engine::new_offline(Store::open_in_memory().unwrap().into_shared());
    engine.execute("sample.load", &json!({"name": "orders"})).unwrap();
    let outside = engine.clone();
    let mut app = PorchlyApp::new(engine, "test.db");
    app.select_table("orders");
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui_state(
        |ui, app: &mut PorchlyApp| {
            PorchlyApp::setup(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    // Order 2 belongs to Ben; move it to Chen.
    // A dropdown reports the chosen row's name as its value.
    h.get_by_value("Ben Okafor").click();
    settle(&mut h);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Chen Wei").click();
    settle(&mut h);
    let row = outside.execute("row.get", &json!({"table": "orders", "id": 2})).unwrap();
    assert_eq!(row["customer"], 3);
    assert!(h.query_by_value("Ben Okafor").is_none());
    assert_eq!(h.query_all_by_value("Chen Wei").count(), 2, "orders 2 and 3 now show Chen");
}

#[test]
fn a_field_is_renamed_from_its_header() {
    let mut h = harness(true);
    h.get_by_label("Change field Minutes").click();
    settle(&mut h);
    assert!(h.query_by_label("Change \u{201c}Minutes\u{201d}").is_some(), "the dialog opens");
    let name =
        h.get_all_by_role(egui::accesskit::Role::TextInput).find(|n| n.value().as_deref() == Some("Minutes")).unwrap();
    name.focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.value().as_deref() == Some("Minutes"))
        .unwrap()
        .type_text("Cooking time");
    settle(&mut h);
    h.get_by_label("Save").click();
    settle(&mut h);
    assert!(h.query_by_label("Change field Cooking time").is_some());
    assert!(h.query_by_label("Change field Minutes").is_none());
}

#[test]
fn a_field_is_deleted_from_its_change_dialog() {
    let mut h = harness(true);
    h.get_by_label("Change field Rating").click();
    settle(&mut h);
    h.get_by_label("Delete field\u{2026}").click();
    settle(&mut h);
    assert!(h.query_by_label("Delete the field \u{201c}Rating\u{201d}?").is_some(), "asks first");
    h.get_by_label("Delete field").click();
    settle(&mut h);
    assert!(h.query_by_label("Change field Rating").is_none());
    assert!(h.query_by_label("Change field Added").is_some(), "other fields stay");
}

#[test]
fn a_failed_change_explains_itself_in_the_dialog() {
    let engine = Engine::new_offline(Store::open_in_memory().unwrap().into_shared());
    engine.execute("sample.load", &json!({})).unwrap();
    let mut app = PorchlyApp::new(engine, "test.db");
    let mut d = porchly_ui_egui::Dialog::edit_field("recipes", "name", porchly_core::FieldKind::Text, true, None);
    if let porchly_ui_egui::Dialog::EditField { kind, .. } = &mut d {
        *kind = porchly_core::FieldKind::Integer;
    }
    app.open_dialog(d);
    app.confirm_dialog();
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui_state(
        |ui, app: &mut PorchlyApp| {
            PorchlyApp::setup(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    assert!(h.query_by_label_contains("6 rows have values that don't fit").is_some());
    assert!(h.query_by_label("Save").is_some(), "the dialog stays open");
}

#[test]
fn a_csv_file_is_imported_into_a_new_table() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("Guest List.csv");
    std::fs::write(&file, "Name,Guests\nAnn,2\nBob,1\nCara,3\n").unwrap();
    let mut h = harness(false);
    h.state_mut().file_dropped(&file);
    settle(&mut h);
    assert!(h.query_by_label("Import \u{201c}Guest List.csv\u{201d}").is_some());
    assert!(h.query_by_label("3 rows, 2 columns.").is_some());
    h.get_by_label("Import").click();
    settle(&mut h);
    assert!(h.query_by_label("3 rows").is_some(), "the new table is shown");
    assert!(h.query_by_label("Change field Guests").is_some());
}

#[test]
fn dropping_something_else_explains_what_works() {
    let mut h = harness(false);
    h.state_mut().file_dropped(std::path::Path::new("/tmp/photo.jpg"));
    settle(&mut h);
    assert!(h.query_by_label_contains("saved as CSV").is_some());
}

/// Clicks into the text box showing `placeholder` and types `text`, like a person would.
fn type_into(h: &mut Harness<'static, PorchlyApp>, placeholder: &str, text: &str) {
    let by_placeholder = |n: &egui_kittest::Node<'_>| n.accesskit_node().placeholder() == Some(placeholder);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(by_placeholder)
        .unwrap_or_else(|| panic!("no text box showing {placeholder:?}"))
        .click();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput).find(by_placeholder).unwrap().type_text(text);
    settle(h);
}

fn harness_with(engine: Engine, table: Option<&str>) -> Harness<'static, PorchlyApp> {
    let mut app = PorchlyApp::new(engine, "test.db");
    if let Some(t) = table {
        app.select_table(t);
    }
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 760.0)).build_ui_state(
        |ui, app: &mut PorchlyApp| {
            PorchlyApp::setup(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    h
}

#[test]
fn the_search_box_finds_rows() {
    let mut h = harness(true);
    type_into(&mut h, "Search this table", "soup");
    assert!(h.query_by_label("1 matching row.").is_some());
    assert!(h.query_all_by_value("Tomato soup").next().is_some());
    assert!(h.query_all_by_value("Fish tacos").next().is_none());
}

#[test]
fn big_tables_are_shown_a_page_at_a_time() {
    let engine = Engine::new_offline(Store::open_in_memory().unwrap().into_shared());
    engine.execute("table.create", &json!({"name": "things", "fields": [{"name": "name", "type": "text"}]})).unwrap();
    for i in 1..=250 {
        engine.execute("row.insert", &json!({"table": "things", "values": {"name": format!("Thing {i}")}})).unwrap();
    }
    let mut h = harness_with(engine, Some("things"));
    assert!(h.query_by_label("Showing 1\u{2013}100 of 250 rows.").is_some());
    h.get_by_label("Next").click();
    settle(&mut h);
    assert!(h.query_by_label("Showing 101\u{2013}200 of 250 rows.").is_some());
    assert!(h.query_all_by_value("Thing 150").next().is_some());
    h.get_by_label("Next").click();
    settle(&mut h);
    assert!(h.query_by_label("Showing 201\u{2013}250 of 250 rows.").is_some());
    h.get_by_label("Previous").click();
    settle(&mut h);
    assert!(h.query_by_label("Showing 101\u{2013}200 of 250 rows.").is_some());
}

#[test]
fn link_dropdowns_search_big_tables() {
    let engine = Engine::new_offline(Store::open_in_memory().unwrap().into_shared());
    engine.execute("sample.load", &json!({"name": "orders"})).unwrap();
    for i in 1..=150 {
        engine.execute("row.insert", &json!({"table": "customers", "values": {"name": format!("Extra {i}")}})).unwrap();
    }
    engine.execute("row.insert", &json!({"table": "customers", "values": {"name": "Zelda Quist"}})).unwrap();
    let outside = engine.clone();
    let mut h = harness_with(engine, Some("orders"));
    // The linked name shows even though Ana is the first customer and lists are limited.
    h.get_by_value("Ben Okafor").click();
    settle(&mut h);
    assert!(h.query_by_label_contains("more: type to search").is_some());
    assert!(h.query_by_role_and_label(egui::accesskit::Role::Button, "Zelda Quist").is_none(), "not in the first 100");
    type_into(&mut h, "Search\u{2026}", "zelda");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Zelda Quist").click();
    settle(&mut h);
    let order = outside.execute("row.get", &json!({"table": "orders", "id": 2})).unwrap();
    assert_eq!(order["customer"], 156, "5 sample customers, 150 extras, then Zelda");
    assert!(h.query_all_by_value("Zelda Quist").next().is_some(), "the cell shows the new name");
}

/// Sends JSON lines to the control channel and returns one answer per line.
fn control_session(port: u16, lines: Vec<serde_json::Value>) -> std::thread::JoinHandle<Vec<serde_json::Value>> {
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Write};
        let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let mut answers = Vec::new();
        for line in lines {
            // The app hangs up on clients without the token; stop sending then.
            if writeln!(out, "{line}").is_err() {
                break;
            }
            let mut reply = String::new();
            if reader.read_line(&mut reply).unwrap_or(0) == 0 {
                break;
            }
            answers.push(serde_json::from_str(&reply).unwrap());
        }
        answers
    })
}

/// Steps the app until the client thread is done (the app answers between frames).
fn run_until_done<T>(h: &mut Harness<'static, PorchlyApp>, job: std::thread::JoinHandle<T>) -> T {
    let start = std::time::Instant::now();
    while !job.is_finished() && start.elapsed() < std::time::Duration::from_secs(15) {
        h.step();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    job.join().unwrap()
}

#[test]
fn the_control_channel_needs_the_token_and_drives_the_app() {
    let mut h = harness(true);
    let ctx = h.ctx.clone();
    let (port, token) = h.state_mut().start_control(0, &ctx).unwrap();

    let wrong =
        control_session(port, vec![json!({"id": 1, "method": "ui.state"}), json!({"id": 2, "method": "ui.state"})]);
    let answers = run_until_done(&mut h, wrong);
    assert_eq!(answers.len(), 1, "the connection is closed after a request without the token");
    assert_eq!(answers[0]["error"]["code"], "unauthorized");

    let bad_token = control_session(port, vec![json!({"id": 1, "method": "auth", "params": {"token": "nope"}})]);
    assert_eq!(run_until_done(&mut h, bad_token)[0]["error"]["code"], "unauthorized");

    let session = control_session(
        port,
        vec![
            json!({"id": 1, "method": "auth", "params": {"token": token}}),
            json!({"id": 2, "method": "engine.execute", "params": {"command": "table.create", "params": {"name": "notes", "fields": [{"name": "text"}]}}}),
            json!({"id": 3, "method": "ui.select", "params": {"table": "notes"}}),
            json!({"id": 4, "method": "engine.execute", "params": {"command": "row.insert", "params": {"table": "notes", "values": {"text": "hi"}}}}),
            json!({"id": 5, "method": "ui.state"}),
            json!({"id": 6, "method": "engine.execute", "params": {"command": "nope.nope"}}),
            json!({"id": 7, "method": "self.destruct"}),
        ],
    );
    let a = run_until_done(&mut h, session);
    assert_eq!(a[0]["result"]["ok"], true);
    assert_eq!(a[1]["result"]["name"], "notes");
    assert_eq!(a[2]["result"]["selected"], "notes");
    assert_eq!(a[4]["result"]["selected"], "notes");
    assert_eq!(a[4]["result"]["total_rows"], 1, "the window shows the row added over the channel");
    assert_eq!(a[5]["error"]["code"], "failed");
    assert!(a[6]["error"]["message"].as_str().unwrap().contains("Unknown method"));
    settle(&mut h);
    assert!(h.query_all_by_value("hi").next().is_some());
}

/// A project on disk (functions need a folder), optionally with functions already on.
fn functions_project(on: bool) -> (tempfile::TempDir, Harness<'static, PorchlyApp>) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path().join("porchly.db")).unwrap();
    if on {
        store.set_setting("functions", Some("on")).unwrap();
    }
    let engine = Engine::new_offline(store.into_shared());
    engine.execute("sample.load", &json!({})).unwrap();
    (dir, harness_with(engine, None))
}

#[test]
fn functions_stay_off_until_wanted() {
    let (dir, mut h) = functions_project(false);
    assert!(h.query_by_label("New function").is_none());
    h.get_by_label("Functions, Off").click();
    settle(&mut h);
    assert!(h.query_by_label("Turn on Functions").is_some(), "explains first");
    h.get_by_label("Not now").click();
    settle(&mut h);
    assert!(h.query_by_label("Turn on Functions").is_none());
    assert!(h.query_by_label("New function").is_none(), "still off");

    // A download that fails says so inside the dialog, with a way to try again.
    h.get_by_label("Functions, Off").click();
    settle(&mut h);
    h.get_by_label("Turn on Functions").click();
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(2);
    }
    assert!(h.query_by_label("Try again").is_some());
    assert!(h.query_by_label_contains("can't be downloaded").is_some());
    assert!(!dir.path().join("tools").exists(), "nothing downloaded");
}

#[test]
fn making_editing_and_deleting_a_function() {
    let (dir, mut h) = functions_project(true);
    assert!(h.query_by_label("No functions yet.").is_some());
    h.get_by_label("New function").click();
    settle(&mut h);
    type_into(&mut h, "For example: Send invoice", "Send invoice");
    assert!(h.query_by_label("Apps will reach it at /api/fn/send_invoice").is_some());
    h.get_by_label("Make function").click();
    settle(&mut h);
    let main = dir.path().join("functions/send_invoice/main.ts");
    assert!(main.is_file());
    assert_eq!(h.query_all_by_label("Send invoice").count(), 2, "listed and open");
    assert!(h.query_by_label("Turn on the API (Share) to reach this function.").is_some());

    // Edit and save.
    assert!(h.query_by_label("Not saved yet").is_none());
    let is_code = |n: &egui_kittest::Node<'_>| n.accesskit_node().value().is_some_and(|v| v.starts_with("// Porchly"));
    h.get_all_by_role(egui::accesskit::Role::MultilineTextInput).find(is_code).expect("the code editor").click();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::MultilineTextInput).find(is_code).unwrap().type_text(" changed ");
    settle(&mut h);
    assert!(h.query_by_label("Not saved yet").is_some());
    h.get_by_label("Save").click();
    settle(&mut h);
    assert!(std::fs::read_to_string(&main).unwrap().contains(" changed "));
    assert!(h.query_by_label("Not saved yet").is_none());

    // Anyone may call it: a warning, and the setting is saved.
    assert!(h.query_by_label_contains("Anyone who knows the address").is_none());
    h.get_by_label("Anyone, no key needed").click();
    settle(&mut h);
    assert!(h.query_by_label_contains("Anyone who knows the address").is_some());
    let settings = std::fs::read_to_string(dir.path().join("functions/send_invoice/function.json")).unwrap();
    assert!(settings.contains("\"public\""), "{settings}");
    assert!(h.query_by_label("Send invoice, no key").is_some(), "the sidebar shows it");

    // Back to a table, then delete the function.
    h.get_by_label("Recipes").click();
    settle(&mut h);
    assert!(h.query_by_label("Add row").is_some());
    h.get_by_label("Send invoice, no key").click();
    settle(&mut h);
    h.get_by_label("Delete\u{2026}").click();
    settle(&mut h);
    h.get_by_label("Delete function").click();
    settle(&mut h);
    assert!(!main.exists());
    assert!(h.query_by_label("No functions yet.").is_some());
    assert!(std::fs::read_dir(dir.path().join("functions/.deleted")).unwrap().count() == 1, "the code is kept");

    // Turning Functions off keeps the code but hides the list.
    h.get_by_label("Turn off").click();
    settle(&mut h);
    h.get_all_by_label("Turn off").last().unwrap().click();
    settle(&mut h);
    assert!(h.query_by_label("New function").is_none());
    assert!(h.query_by_label("Functions, Off").is_some(), "greyed out again");
}

#[test]
fn the_porch_light_warns_first_and_reports_problems() {
    let (_dir, mut h) = functions_project(false);
    h.get_by_label("Share").click();
    settle(&mut h);
    assert!(h.query_by_label_contains("Only this computer can reach your API").is_some());
    h.get_by_label("Turn on the porch light\u{2026}").click();
    settle(&mut h);
    assert!(h.query_by_label("Turn on the porch light?").is_some(), "explains first");
    assert!(h.query_by_label_contains("Anyone with the address and one of your keys").is_some());
    let allow = h.get_by_label("Allow changes through the link");
    assert_eq!(allow.accesskit_node().toggled(), Some(egui::accesskit::Toggled::False), "read-only by default");
    h.get_by_label("Cancel").click();
    settle(&mut h);
    assert!(h.query_by_label("Turn on the porch light?").is_none());

    // This test engine may not open ports or download, so turning on fails, and says so.
    h.get_by_label("Turn on the porch light\u{2026}").click();
    settle(&mut h);
    // The API has a "Turn on" too; the dialog's is drawn last.
    h.get_all_by_label("Turn on").last().unwrap().click();
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        h.run_steps(2);
    }
    assert!(h.query_by_label_contains("can't be").is_some(), "the problem is shown");
    assert!(h.query_by_label("Turn on the porch light\u{2026}").is_some(), "still off");
}

#[test]
fn settings_are_simple_choices() {
    let (dir, mut h) = functions_project(false);
    h.get_by_label("Settings").click();
    settle(&mut h);
    assert!(h.query_by_label("Requests per second, per app").is_some());
    assert!(h.query_by_label_contains("Stop a function").is_none(), "no function settings while they're off");
    let rate = |n: &egui_kittest::Node<'_>| n.accesskit_node().value().as_deref() == Some("20");
    h.get_all_by_role(egui::accesskit::Role::TextInput).find(rate).unwrap().click();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput).find(rate).unwrap().type_text("0");
    settle(&mut h);
    h.get_by_label("Turn on the API when Porchly opens").click();
    settle(&mut h);
    h.get_by_label("Save").click();
    settle(&mut h);
    assert!(h.query_by_label("Requests per second, per app").is_none(), "closed");

    // Saved with the project.
    let e = Engine::new_offline(Store::open(dir.path().join("porchly.db")).unwrap().into_shared());
    let s = e.execute("settings.get", &json!({})).unwrap();
    // "0" typed into "20" lands before or after the cursor: 200 or 020.
    assert!([20, 200].contains(&s["requests_per_second"].as_u64().unwrap()), "{s}");
    assert_eq!(s["start_api"].as_bool(), Some(false));

    // A number that isn't usable can't be saved, and says why.
    h.get_by_label("Settings").click();
    settle(&mut h);
    let port = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.accesskit_node().value().as_deref() == Some("11911"))
        .unwrap();
    port.click();
    h.run_steps(2);
    let port = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.accesskit_node().value().as_deref() == Some("11911"))
        .unwrap();
    port.type_text("x");
    settle(&mut h);
    assert!(h.query_by_label_contains("from 1024 to 65535").is_some(), "explains why");
}
