//! Sample projects, defined in code (original content, no asset files).

use porchly_core::{Error, FieldKind, Ident, Result};
use porchly_schema::FieldDef;
use serde_json::{Value, json};

use super::{Command, always, to_json};
use crate::Ctx;
use crate::params::Params;

pub(super) fn commands() -> Vec<Command> {
    vec![Command {
        id: "sample.load",
        label: "Add a sample table",
        menu: "Help/Add a sample table",
        shortcut: None,
        params: "name?: \"recipes\" (the default) or \"orders\" (customers and their orders, linked)",
        enabled: always,
        run: load,
    }]
}

fn load(cx: &mut Ctx<'_>, params: &Value) -> Result<Value> {
    let p = Params::new(params)?;
    match p.opt_str("name")?.unwrap_or("recipes") {
        "recipes" => recipes(cx),
        "orders" => orders(cx),
        other => Err(Error::invalid(format!(
            "There's no sample called \"{}\". Try \"recipes\" or \"orders\".",
            other.chars().take(40).collect::<String>()
        ))),
    }
}

fn recipes(cx: &mut Ctx<'_>) -> Result<Value> {
    let f = |name: &str, kind, required| -> Result<FieldDef> {
        Ok(FieldDef { name: Ident::new(name)?, kind, required, links_to: None })
    };
    let fields = vec![
        f("name", FieldKind::Text, true)?,
        f("minutes", FieldKind::Integer, false)?,
        f("serves", FieldKind::Integer, false)?,
        f("vegetarian", FieldKind::Boolean, false)?,
        f("rating", FieldKind::Number, false)?,
        f("added", FieldKind::Datetime, false)?,
    ];
    let store = cx.store();
    let table = store.create_table(&Ident::new("recipes")?, fields)?;
    let rows = [
        json!({"name": "Buttermilk pancakes", "minutes": 25, "serves": 4, "vegetarian": true, "rating": 4.5, "added": "2026-09-14"}),
        json!({"name": "Three-bean chili", "minutes": 60, "serves": 6, "vegetarian": true, "rating": 4.8, "added": "2026-09-20"}),
        json!({"name": "Lemon roast chicken", "minutes": 90, "serves": 4, "vegetarian": false, "rating": 4.7, "added": "2026-09-22"}),
        json!({"name": "Tomato soup", "minutes": 35, "serves": 4, "vegetarian": true, "rating": 4.1, "added": "2026-09-30"}),
        json!({"name": "Fish tacos", "minutes": 30, "serves": 3, "vegetarian": false, "rating": 4.4, "added": "2026-10-02"}),
        json!({"name": "Apple crumble", "minutes": 50, "serves": 8, "vegetarian": true, "rating": null, "added": "2026-10-05"}),
    ];
    for row in &rows {
        if let Some(values) = row.as_object() {
            store.insert_row("recipes", values)?;
        }
    }
    to_json(table)
}

/// Two linked tables: customers, and orders that link to them.
fn orders(cx: &mut Ctx<'_>) -> Result<Value> {
    let store = cx.store();
    let text = |n: &str| -> Result<FieldDef> { Ok(FieldDef::new(Ident::new(n)?, FieldKind::Text, false)) };
    store.create_table(
        &Ident::new("customers")?,
        vec![FieldDef::new(Ident::new("name")?, FieldKind::Text, true), text("city")?, text("email")?],
    )?;
    let table = store.create_table(
        &Ident::new("orders")?,
        vec![
            FieldDef::new(Ident::new("item")?, FieldKind::Text, true),
            FieldDef::new(Ident::new("quantity")?, FieldKind::Integer, false),
            FieldDef::link(Ident::new("customer")?, Ident::new("customers")?, false),
            FieldDef::new(Ident::new("delivered")?, FieldKind::Boolean, false),
        ],
    )?;
    let customers = [
        json!({"name": "Ana Lima", "city": "Lisbon", "email": "ana@example.com"}),
        json!({"name": "Ben Okafor", "city": "Leeds", "email": "ben@example.com"}),
        json!({"name": "Chen Wei", "city": "Lisbon", "email": "chen@example.com"}),
        json!({"name": "Dana Fischer", "city": "Berlin", "email": "dana@example.com"}),
        json!({"name": "Eli Moreau", "city": "Lyon", "email": "eli@example.com"}),
    ];
    let orders = [
        json!({"item": "Sourdough loaf", "quantity": 2, "customer": 1, "delivered": true}),
        json!({"item": "Jar of honey", "quantity": 1, "customer": 2, "delivered": false}),
        json!({"item": "Rye crackers", "quantity": 3, "customer": 3, "delivered": true}),
        json!({"item": "Olive oil", "quantity": 1, "customer": 1, "delivered": false}),
        json!({"item": "Coffee beans", "quantity": 1, "customer": 4, "delivered": true}),
        json!({"item": "Strawberry jam", "quantity": 2, "customer": 5, "delivered": false}),
        json!({"item": "Granola", "quantity": 1, "customer": 4, "delivered": false}),
        json!({"item": "Sea salt", "quantity": 2, "customer": 5, "delivered": true}),
        json!({"item": "Earl Grey tea", "quantity": 1, "customer": 4, "delivered": true}),
    ];
    for row in customers.iter().filter_map(Value::as_object) {
        store.insert_row("customers", row)?;
    }
    for row in orders.iter().filter_map(Value::as_object) {
        store.insert_row("orders", row)?;
    }
    to_json(table)
}
