#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use axum::http::{Method, StatusCode};
use common::app;

#[tokio::test]
async fn health_needs_no_key() {
    let a = app();
    let (status, body) = a.send(Method::GET, "/health", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
}

#[tokio::test]
async fn index_lists_tables() {
    let a = app();
    let (status, body) = a.get("/api").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["tables"][0]["name"], "recipes");
    assert_eq!(body["tables"][0]["fields"][1]["type"], "integer");
}

#[tokio::test]
async fn crud_round_trip() {
    let a = app();
    let (status, row) = a.post("/api/recipes", r#"{"name":"Pancakes","minutes":20,"vegetarian":true}"#).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(row["id"], 1);
    assert_eq!(row["vegetarian"], true);

    let (status, row) = a.get("/api/recipes/1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row["name"], "Pancakes");

    let key = a.write_key.clone();
    let (status, row) = a.send(Method::PATCH, "/api/recipes/1", Some(&key), Some(r#"{"minutes":25}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row["minutes"], 25);

    let (status, body) = a.send(Method::DELETE, "/api/recipes/1", Some(&key), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_null());

    let (status, body) = a.get("/api/recipes/1").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"]["code"], "not_found");
}

#[tokio::test]
async fn list_with_filters_sort_and_paging() {
    let a = app();
    for (name, minutes, veg) in [("Chili", 45, false), ("Pancakes", 20, true), ("Salad", 10, true)] {
        let body = format!(r#"{{"name":"{name}","minutes":{minutes},"vegetarian":{veg}}}"#);
        assert_eq!(a.post("/api/recipes", &body).await.0, StatusCode::CREATED);
    }
    let (status, page) = a.get("/api/recipes?vegetarian=true&sort=-minutes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["total"], 2);
    assert_eq!(page["items"][0]["name"], "Pancakes");

    let (_, page) = a.get("/api/recipes?limit=1&offset=2").await;
    assert_eq!(page["items"][0]["name"], "Salad");
    assert_eq!(page["limit"], 1);

    let (_, page) = a.get("/api/recipes?name=Salad%20").await;
    assert_eq!(page["total"], 0, "filters are exact");
}

#[tokio::test]
async fn validation_errors_are_friendly() {
    let a = app();
    let (status, body) = a.post("/api/recipes", r#"{"minutes":20}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "invalid");
    assert!(body["error"]["message"].as_str().unwrap().contains("name"));

    let (status, _) = a.post("/api/recipes", r#"{"name":"x","minutes":"lots"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = a.post("/api/recipes", "not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = a.post("/api/recipes", "[1,2]").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn unknown_routes_and_methods() {
    let a = app();
    assert_eq!(a.get("/nope").await.0, StatusCode::NOT_FOUND);
    assert_eq!(a.get("/api/missing_table").await.0, StatusCode::NOT_FOUND);
    let key = a.write_key.clone();
    let (status, body) = a.send(Method::PUT, "/api/recipes", Some(&key), Some("{}")).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body["error"]["code"], "method_not_allowed");
}

#[tokio::test]
async fn openapi_describes_tables() {
    let a = app();
    let (status, spec) = a.get("/api/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(spec["openapi"], "3.1.0");
    assert!(spec["paths"]["/api/recipes"]["get"].is_object());
    assert!(spec["paths"]["/api/recipes/{id}"]["patch"].is_object());
    assert_eq!(spec["components"]["schemas"]["recipes"]["required"][0], "name");
    assert_eq!(spec["components"]["schemas"]["recipes"]["properties"]["minutes"]["type"][1], "null");
    assert_eq!(a.send(Method::GET, "/api/openapi.json", None, None).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn docs_page_needs_a_key_and_describes_tables() {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let a = app();
    assert_eq!(a.send(Method::GET, "/docs", None, None).await.0, StatusCode::UNAUTHORIZED);
    let req = Request::get("/docs")
        .header("authorization", format!("Bearer {}", a.read_key))
        .header("host", "127.0.0.1:11911")
        .body(Body::empty())
        .unwrap();
    let res = a.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    let page = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    assert!(page.contains("id=\"t-recipes\"") && page.contains("http://127.0.0.1:11911/api/recipes"));
}

#[tokio::test]
async fn rows_keep_field_order() {
    let a = app();
    let (_, row) = a.post("/api/recipes", r#"{"vegetarian":true,"name":"Soup"}"#).await;
    let keys: Vec<&String> = row.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["id", "created_at", "updated_at", "name", "minutes", "vegetarian"]);
}

#[tokio::test]
async fn filter_operators_over_http() {
    let a = app();
    for (name, minutes, veg) in
        [("Chili", "45", "false"), ("Pancakes", "20", "true"), ("Salad", "10", "null"), ("Tomato soup", "35", "true")]
    {
        let body = format!(r#"{{"name":"{name}","minutes":{minutes},"vegetarian":{veg}}}"#);
        assert_eq!(a.post("/api/recipes", &body).await.0, StatusCode::CREATED);
    }
    let names = |page: &serde_json::Value| -> Vec<String> {
        page["items"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_owned()).collect()
    };
    let (status, page) = a.get("/api/recipes?minutes=lt.30&sort=minutes").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&page), ["Salad", "Pancakes"]);
    let (_, page) = a.get("/api/recipes?name=contains.SOUP").await;
    assert_eq!(names(&page), ["Tomato soup"]);
    let (_, page) = a.get("/api/recipes?minutes=in.10,45").await;
    assert_eq!(names(&page), ["Chili", "Salad"]);
    let (_, page) = a.get("/api/recipes?vegetarian=is.empty").await;
    assert_eq!(names(&page), ["Salad"]);
    let (_, page) = a.get("/api/recipes?minutes=gte.20&minutes=lte.40").await;
    assert_eq!(names(&page), ["Pancakes", "Tomato soup"]);
    let (_, page) = a.get("/api/recipes?name=starts.pan&vegetarian=true").await;
    assert_eq!(page["total"], 1);

    let (status, body) = a.get("/api/recipes?minutes=contains.3").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().contains("contains"), "{body}");
}

/// customers ← orders.customer, through the real HTTP routes.
#[tokio::test]
async fn joins_over_http() {
    let a = app();
    {
        let mut s = porchly_store::lock(&a.store);
        let f = |n: &str| {
            porchly_schema::FieldDef::new(porchly_core::Ident::new(n).unwrap(), porchly_core::FieldKind::Text, false)
        };
        s.create_table(&porchly_core::Ident::new("customers").unwrap(), vec![f("name"), f("city")]).unwrap();
        s.add_field(
            "recipes",
            porchly_schema::FieldDef::link(
                porchly_core::Ident::new("cook").unwrap(),
                porchly_core::Ident::new("customers").unwrap(),
                false,
            ),
        )
        .unwrap();
    }
    a.post("/api/customers", r#"{"name":"Ann","city":"Paris"}"#).await;
    a.post("/api/customers", r#"{"name":"Bob","city":"Oslo"}"#).await;
    a.post("/api/recipes", r#"{"name":"Crepes","cook":1}"#).await;
    a.post("/api/recipes", r#"{"name":"Lutefisk","cook":2}"#).await;

    let (status, page) = a.get("/api/recipes?cook.city=Paris&expand=cook").await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["name"], "Crepes");
    assert_eq!(page["items"][0]["expand"]["cook"]["name"], "Ann");

    let (_, page) = a.get("/api/recipes?sort=-cook.name").await;
    assert_eq!(page["items"][0]["name"], "Lutefisk");

    let (status, row) = a.get("/api/recipes/2?expand=cook").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row["expand"]["cook"]["city"], "Oslo");
    assert_eq!(a.get("/api/recipes/2?sort=name").await.0, StatusCode::BAD_REQUEST, "single rows only take expand");

    let (status, body) = a.post("/api/recipes", r#"{"name":"Ghost","cook":42}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().contains("no row 42"));

    let key = a.write_key.clone();
    assert_eq!(a.send(Method::DELETE, "/api/customers/1", Some(&key), None).await.0, StatusCode::NO_CONTENT);
    let (_, row) = a.get("/api/recipes/1").await;
    assert!(row["cook"].is_null(), "deleting the cook clears the link and keeps the recipe");

    let (_, spec) = a.get("/api/openapi.json").await;
    assert_eq!(
        spec["components"]["schemas"]["recipes"]["properties"]["cook"]["description"],
        "The id of a row in customers."
    );
    assert!(spec["components"]["schemas"]["recipes"]["properties"]["expand"]["properties"]["cook"].is_object());
}

#[tokio::test]
async fn bulk_add_change_and_delete() {
    let a = app();
    let key = a.write_key.clone();
    // Add several rows at once.
    let (status, body) = a
        .post("/api/recipes", r#"[{"name":"Soup","minutes":30},{"name":"Salad","minutes":10,"vegetarian":true},{"name":"Stew","minutes":90}]"#)
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["count"], 3);
    assert_eq!((body["items"][0]["id"].as_i64(), body["items"][2]["name"].as_str()), (Some(1), Some("Stew")));

    // One bad row: nothing is added, and the error says which row.
    let (status, body) = a.post("/api/recipes", r#"[{"name":"Ok"},{"minutes":5}]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"].as_str().unwrap().starts_with("Row 2:"), "{body}");
    let (status, _) = a.post("/api/recipes", r#"[{"name":"Ok"}, 7]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(a.row_count(), 3);

    // Change every row matching a condition.
    let (status, body) =
        a.send(Method::PATCH, "/api/recipes?minutes=lt.60", Some(&key), Some(r#"{"vegetarian":true}"#)).await;
    assert_eq!((status, body["updated"].as_u64()), (StatusCode::OK, Some(2)));
    let (_, page) = a.get("/api/recipes?vegetarian=true").await;
    assert_eq!(page["total"], 2);

    // Delete by ids, then by a condition.
    let (status, body) = a.send(Method::DELETE, "/api/recipes?id=in.1,3", Some(&key), None).await;
    assert_eq!((status, body["deleted"].as_u64()), (StatusCode::OK, Some(2)));
    let (_, body) = a.send(Method::DELETE, "/api/recipes?name=Nobody", Some(&key), None).await;
    assert_eq!(body["deleted"], 0);
    assert_eq!(a.row_count(), 1);
}
