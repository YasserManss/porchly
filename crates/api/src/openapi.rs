//! An OpenAPI 3.1 description of the project's API, generated from its tables. Tools,
//! code generators and AI agents read it to learn what the API offers.

use porchly_core::{FieldKind, limits};
use porchly_schema::TableDef;

use crate::{FnAccess, FnSummary};
use serde_json::{Map, Value, json};

/// How a field filter is written, shown for every field in the description.
const FILTER_HELP: &str = "Only rows matching this condition. A plain value means equals (minutes=20). Or write op.value: ne, lt, lte, gt, gte (minutes=lt.30), contains, starts, ends (text only, ignores case: name=contains.soup), in (serves=in.2,4), is.empty / is.notempty. Use eq. to match a value that starts with an operator name (name=eq.in.time).";

pub(crate) fn spec(tables: &[TableDef], functions: &[FnSummary]) -> Value {
    let mut paths = Map::new();
    let mut schemas = Map::new();
    schemas.insert(
        "Error".into(),
        json!({
            "type": "object",
            "properties": { "error": { "type": "object", "properties": {
                "code": { "type": "string" }, "message": { "type": "string" } } } }
        }),
    );
    for t in tables {
        let name = t.name.as_str();
        let row_ref = json!({ "$ref": format!("#/components/schemas/{name}") });
        schemas.insert(name.to_owned(), row_schema(t));

        let mut list_params = vec![
            query_param(
                "limit",
                json!({"type": "integer", "minimum": 0, "maximum": limits::MAX_PAGE}),
                "Rows per page.",
            ),
            query_param("offset", json!({"type": "integer", "minimum": 0}), "Rows to skip."),
            query_param(
                "sort",
                json!({"type": "string"}),
                "A field to sort by; prefix with - for descending. Fields of linked tables work too (customer.name).",
            ),
        ];
        list_params.push(query_param(
            "search",
            json!({"type": "string"}),
            "Text to look for in any text field, ignoring case (a number also matches that row's id).",
        ));
        let links: Vec<&str> = t.fields.iter().filter(|f| f.kind == FieldKind::Link).map(|f| f.name.as_str()).collect();
        let expand_param = (!links.is_empty()).then(|| {
            query_param(
                "expand",
                json!({"type": "string"}),
                &format!("Link fields whose rows to embed under \"expand\", comma-separated: {}.", links.join(", ")),
            )
        });
        list_params.extend(expand_param.clone());
        for f in &t.fields {
            list_params.push(query_param(f.name.as_str(), json!({ "type": "string" }), FILTER_HELP));
        }
        if !links.is_empty() {
            list_params.push(json!({
                "name": "link.field",
                "in": "query",
                "required": false,
                "schema": { "type": "string" },
                "description": format!(
                    "A condition on a field of a linked table, written link.field (for example {}.name=contains.ann). Same operators as above.",
                    links.first().copied().unwrap_or("customer")
                )
            }));
        }
        // Bulk changes take the conditions of a list, but none of its paging.
        let mut bulk_params = vec![
            query_param("id", json!({ "type": "string" }), "Rows by id, for example in.1,2,3."),
            query_param("search", json!({ "type": "string" }), "Rows with this text in any text field."),
        ];
        for f in &t.fields {
            bulk_params.push(query_param(f.name.as_str(), json!({ "type": "string" }), FILTER_HELP));
        }
        paths.insert(
            format!("/api/{name}"),
            json!({
                "get": {
                    "summary": format!("List {name}"),
                    "parameters": list_params,
                    "responses": {
                        "200": ok(json!({ "type": "object", "properties": {
                            "items": { "type": "array", "items": row_ref },
                            "limit": { "type": "integer" }, "offset": { "type": "integer" }, "total": { "type": "integer" } } })),
                        "default": error()
                    }
                },
                "post": {
                    "summary": format!("Add a row to {name}, or several"),
                    "description": format!("Send one row, or a list of up to {} rows. A list is all or nothing: if one row is refused, none are added and the error names the row. A list answers with {{count, items}}.", limits::MAX_BULK_ROWS),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": {
                        "oneOf": [row_ref, { "type": "array", "items": row_ref, "maxItems": limits::MAX_BULK_ROWS }] } } } },
                    "responses": {
                        "201": ok(json!({ "oneOf": [row_ref, { "type": "object", "properties": {
                            "count": { "type": "integer" }, "items": { "type": "array", "items": row_ref } } }] })),
                        "default": error()
                    }
                },
                "patch": {
                    "summary": format!("Change every row of {name} that matches"),
                    "description": "Every row matching the conditions gets the fields sent. At least one condition is required (for example id=in.1,2,3); limit, offset, sort and expand aren't allowed.",
                    "parameters": bulk_params.clone(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": row_ref } } },
                    "responses": {
                        "200": ok(json!({ "type": "object", "properties": { "updated": { "type": "integer" } } })),
                        "default": error()
                    }
                },
                "delete": {
                    "summary": format!("Delete every row of {name} that matches"),
                    "description": "Every row matching the conditions is deleted; links to them are cleared. At least one condition is required (for example id=in.1,2,3); limit, offset, sort and expand aren't allowed.",
                    "parameters": bulk_params,
                    "responses": {
                        "200": ok(json!({ "type": "object", "properties": { "deleted": { "type": "integer" } } })),
                        "default": error()
                    }
                }
            }),
        );
        let id_param = json!({ "name": "id", "in": "path", "required": true, "schema": { "type": "integer" } });
        let mut one_params = vec![id_param];
        one_params.extend(expand_param);
        paths.insert(
            format!("/api/{name}/{{id}}"),
            json!({
                "parameters": one_params,
                "get": { "summary": format!("Get one row of {name}"), "responses": { "200": ok(row_ref.clone()), "default": error() } },
                "patch": {
                    "summary": format!("Change a row of {name}"),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": row_ref } } },
                    "responses": { "200": ok(row_ref.clone()), "default": error() }
                },
                "delete": { "summary": format!("Delete a row of {name}"), "responses": { "204": { "description": "Deleted" }, "default": error() } }
            }),
        );
    }
    for f in functions {
        let name = &f.name;
        let mut op = json!({
            "summary": format!("Run the function {name}"),
            "description": "A function written by the owner of this Porchly. It decides what it accepts and returns. Paths below this address reach it too.",
            "responses": { "default": { "description": "Whatever the function returns." } }
        });
        if f.access == FnAccess::Public
            && let Some(o) = op.as_object_mut()
        {
            o.insert("security".into(), json!([]));
        }
        let methods: Map<String, Value> =
            ["get", "post", "put", "patch", "delete"].iter().map(|m| ((*m).to_owned(), op.clone())).collect();
        paths.insert(format!("/api/fn/{name}"), Value::Object(methods));
    }
    json!({
        "openapi": "3.1.0",
        "info": { "title": "Porchly API", "version": env!("CARGO_PKG_VERSION") },
        "security": [{ "key": [] }],
        "paths": paths,
        "components": {
            "securitySchemes": { "key": { "type": "http", "scheme": "bearer", "description": "A Porchly key (pk_…)." } },
            "schemas": schemas
        }
    })
}

fn row_schema(t: &TableDef) -> Value {
    let mut props = Map::new();
    props.insert("id".into(), json!({ "type": "integer", "readOnly": true }));
    props.insert("created_at".into(), json!({ "type": "string", "format": "date-time", "readOnly": true }));
    props.insert("updated_at".into(), json!({ "type": "string", "format": "date-time", "readOnly": true }));
    for f in &t.fields {
        let mut s = kind_schema(f.kind);
        if let (Some(target), Some(obj)) = (&f.links_to, s.as_object_mut()) {
            obj.insert("description".into(), json!(format!("The id of a row in {target}.")));
        }
        if !f.required
            && let Some(obj) = s.as_object_mut()
        {
            let ty = obj.remove("type").unwrap_or(Value::Null);
            obj.insert("type".into(), json!([ty, "null"]));
        }
        props.insert(f.name.as_str().to_owned(), s);
    }
    let links: Map<String, Value> = t
        .fields
        .iter()
        .filter_map(|f| {
            f.links_to
                .as_ref()
                .map(|target| (f.name.as_str().to_owned(), json!({ "$ref": format!("#/components/schemas/{target}") })))
        })
        .collect();
    if !links.is_empty() {
        props.insert(
            "expand".into(),
            json!({ "type": "object", "readOnly": true, "description": "Linked rows, when asked for with ?expand=.", "properties": links }),
        );
    }
    let required: Vec<&str> = t.fields.iter().filter(|f| f.required).map(|f| f.name.as_str()).collect();
    json!({ "type": "object", "properties": props, "required": required, "additionalProperties": false })
}

fn kind_schema(kind: FieldKind) -> Value {
    match kind {
        FieldKind::Text => json!({ "type": "string" }),
        FieldKind::Integer => json!({ "type": "integer" }),
        FieldKind::Number => json!({ "type": "number" }),
        FieldKind::Boolean => json!({ "type": "boolean" }),
        FieldKind::Datetime => json!({ "type": "string", "format": "date-time" }),
        FieldKind::Link => json!({ "type": "integer", "minimum": 1 }),
    }
}

fn query_param(name: &str, schema: Value, description: &str) -> Value {
    json!({ "name": name, "in": "query", "required": false, "schema": schema, "description": description })
}

fn ok(schema: Value) -> Value {
    json!({ "description": "OK", "content": { "application/json": { "schema": schema } } })
}

fn error() -> Value {
    json!({ "description": "Error", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } } })
}
