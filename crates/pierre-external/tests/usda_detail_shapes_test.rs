// ABOUTME: Drives UsdaClient::get_food_details against a local fixture of the three
// ABOUTME: foodNutrients shapes USDA has shipped: nested, flat, and identity-less
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /fdc/v1/food/{fdcId}` has served its `foodNutrients` entries in two
//! documented shapes and, once, in a third that named no nutrient at all.
//! The client asks for the nested shape explicitly, reads both documented
//! shapes, and turns the third into an error instead of a food whose every
//! nutrient lookup misses (carnet#423). The fixture records the query string
//! so the `format=full` request is asserted, not assumed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use pierre_external::{UsdaClient, UsdaClientConfig};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// The fixture: one canned detail body plus the query string of the last
/// request it answered.
#[derive(Clone)]
struct Fixture {
    body: Value,
    last_query: Arc<Mutex<HashMap<String, String>>>,
}

async fn food_detail(
    State(fixture): State<Fixture>,
    Path(fdc_id): Path<u64>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    *fixture.last_query.lock().unwrap() = query;
    let mut body = fixture.body;
    body["fdcId"] = json!(fdc_id);
    Json(body)
}

async fn serve(body: Value) -> (UsdaClient, Fixture) {
    let fixture = Fixture {
        body,
        last_query: Arc::new(Mutex::new(HashMap::new())),
    };
    let router = Router::new()
        .route("/food/{fdc_id}", get(food_detail))
        .with_state(fixture.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = UsdaClient::new(UsdaClientConfig {
        api_key: "test-key".to_owned(),
        base_url: format!("http://{addr}"),
        ..UsdaClientConfig::default()
    });
    (client, fixture)
}

fn detail(food_nutrients: &Value) -> Value {
    json!({
        "fdcId": 0,
        "description": "Chicken, breast, meat only, cooked, roasted",
        "dataType": "SR Legacy",
        "foodNutrients": food_nutrients,
        "servingSize": 100.0,
        "servingSizeUnit": "g",
    })
}

#[tokio::test]
async fn nested_shape_parses_and_the_request_asks_for_full_format() {
    let (client, fixture) = serve(detail(&json!([
        {"type": "FoodNutrient", "id": 2_650_307, "amount": 165.0,
         "nutrient": {"id": 1008, "number": "208", "name": "Energy", "unitName": "kcal"}},
        {"type": "FoodNutrient", "id": 2_650_301, "amount": 31.02,
         "nutrient": {"id": 1003, "number": "203", "name": "Protein", "unitName": "g"}},
    ])))
    .await;

    let food = client
        .get_food_details(171_477)
        .await
        .expect("nested shape parses");

    assert_eq!(food.fdc_id, 171_477);
    assert_eq!(food.food_nutrients.len(), 2);
    let energy = food
        .food_nutrients
        .iter()
        .find(|n| n.nutrient_id == 1008)
        .expect("energy resolved by nutrient id");
    assert_eq!(energy.nutrient_name, "Energy");
    assert_eq!(energy.unit_name, "kcal");
    assert!((energy.amount - 165.0).abs() < f64::EPSILON);

    let query = fixture.last_query.lock().unwrap().clone();
    assert_eq!(query.get("format").map(String::as_str), Some("full"));
    assert_eq!(query.get("api_key").map(String::as_str), Some("test-key"));
}

#[tokio::test]
async fn flat_shape_resolves_by_nutrient_id_or_number() {
    let (client, _fixture) = serve(detail(&json!([
        {"nutrientId": 1008, "nutrientName": "Energy", "unitName": "kcal", "amount": 165.0},
        {"nutrientNumber": "203", "nutrientName": "Protein", "unitName": "g", "amount": 31.02},
        {"nutrientNumber": "999", "nutrientName": "Unknown", "unitName": "g", "amount": 1.0},
    ])))
    .await;

    let food = client
        .get_food_details(171_477)
        .await
        .expect("flat shape parses");

    let ids: Vec<u32> = food.food_nutrients.iter().map(|n| n.nutrient_id).collect();
    assert_eq!(
        ids,
        vec![1008, 1003],
        "id direct, number mapped, unknown number dropped"
    );
    let protein = &food.food_nutrients[1];
    assert_eq!(protein.nutrient_name, "Protein");
    assert!((protein.amount - 31.02).abs() < f64::EPSILON);
}

#[tokio::test]
async fn entries_naming_no_nutrient_are_an_error_not_a_zero() {
    let (client, _fixture) = serve(detail(&json!([
        {"amount": 165.0, "id": 2_650_307, "type": "FoodNutrient"},
        {"amount": 31.02, "id": 2_650_301, "type": "FoodNutrient"},
    ])))
    .await;

    let err = client
        .get_food_details(171_477)
        .await
        .expect_err("a nutrient list that names no nutrient must fail closed");
    let message = err.to_string();
    assert!(message.contains("171477"), "names the food: {message}");
    assert!(
        message.contains("2 foodNutrients"),
        "counts the entries: {message}"
    );
}

#[tokio::test]
async fn empty_nutrient_list_is_a_food_with_no_nutrients() {
    let (client, _fixture) = serve(detail(&json!([]))).await;

    let food = client
        .get_food_details(171_477)
        .await
        .expect("an empty list is a legitimate answer");
    assert!(food.food_nutrients.is_empty());
}
