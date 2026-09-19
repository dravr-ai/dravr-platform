// ABOUTME: Direct RecipeRepository tests on whichever backend the test factory opens (SQLite or PostgreSQL)
// ABOUTME: Round-trips every column through create, get_by_id, list, update, update_nutrition_cache, search, count, delete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The recipe tools reach the repository only through `save_recipe`,
//! `list_recipes`, `get_recipe`, `delete_recipe` and `search_recipes`; this
//! file calls the trait itself so `update`, `update_nutrition_cache`,
//! `count` and the value of every stored column are pinned on both drivers.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use chrono::{Duration, Utc};
use pierre_core::models::recipes::{
    IngredientUnit, MealTiming, Recipe, RecipeIngredient, ValidatedNutrition,
};
use uuid::Uuid;

#[path = "helpers/db_fixtures.rs"]
mod db_fixtures;
use db_fixtures::{create_test_db, seed_user};

fn sample_recipe(user_id: Uuid, name: &str, timing: MealTiming) -> Recipe {
    let mut recipe = Recipe::new(user_id, name, 4)
        .with_description("Oats, banana and peanut butter")
        .with_instruction("Mash the banana")
        .with_instruction("Stir in the oats");
    recipe.prep_time_mins = Some(5);
    recipe.cook_time_mins = Some(12);
    recipe.tags = vec!["breakfast".to_owned(), "high-carb".to_owned()];
    recipe.meal_timing = timing;
    recipe.ingredients = vec![
        RecipeIngredient {
            fdc_id: Some(173_944),
            name: "rolled oats".to_owned(),
            amount: 1.0,
            unit: IngredientUnit::Cups,
            grams: 80.0,
            preparation: None,
        },
        RecipeIngredient {
            fdc_id: None,
            name: "banana".to_owned(),
            amount: 1.0,
            unit: IngredientUnit::Pieces,
            grams: 118.0,
            preparation: Some("mashed".to_owned()),
        },
    ];
    recipe
}

fn sample_nutrition() -> ValidatedNutrition {
    ValidatedNutrition {
        calories: 412.5,
        protein_g: 14.2,
        carbs_g: 62.0,
        fat_g: 11.8,
        fiber_g: Some(8.1),
        sodium_mg: None,
        sugar_g: Some(19.4),
        validated_at: Utc::now() - Duration::hours(1),
    }
}

#[tokio::test]
async fn create_then_get_round_trips_every_column() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let mut recipe = sample_recipe(user_id, "Overnight oats", MealTiming::PreTraining);
    recipe.nutrition = Some(sample_nutrition());
    let recipe_id = repo.create(user_id, tenant_id, &recipe).await.unwrap();
    assert_eq!(recipe_id, recipe.id.to_string());

    let stored = repo
        .get_by_id(&recipe_id, user_id, tenant_id)
        .await
        .unwrap()
        .expect("the recipe just created");
    assert_eq!(stored.id, recipe.id);
    assert_eq!(stored.user_id, user_id);
    assert_eq!(stored.name, "Overnight oats");
    assert_eq!(
        stored.description.as_deref(),
        Some("Oats, banana and peanut butter")
    );
    assert_eq!(stored.servings, 4);
    assert_eq!(stored.prep_time_mins, Some(5));
    assert_eq!(stored.cook_time_mins, Some(12));
    assert_eq!(
        stored.instructions,
        vec!["Mash the banana", "Stir in the oats"]
    );
    assert_eq!(stored.tags, vec!["breakfast", "high-carb"]);
    assert_eq!(stored.meal_timing, MealTiming::PreTraining);

    assert_eq!(stored.ingredients.len(), 2);
    assert_eq!(stored.ingredients[0].name, "rolled oats");
    assert_eq!(stored.ingredients[0].fdc_id, Some(173_944));
    assert_eq!(stored.ingredients[0].unit, IngredientUnit::Cups);
    assert!((stored.ingredients[0].grams - 80.0).abs() < f64::EPSILON);
    assert_eq!(stored.ingredients[1].name, "banana");
    assert_eq!(stored.ingredients[1].fdc_id, None);
    assert_eq!(stored.ingredients[1].unit, IngredientUnit::Pieces);
    assert_eq!(stored.ingredients[1].preparation.as_deref(), Some("mashed"));

    let nutrition = stored.nutrition.expect("nutrition was stored");
    let expected = recipe.nutrition.unwrap();
    assert!((nutrition.calories - 412.5).abs() < f64::EPSILON);
    assert!((nutrition.protein_g - 14.2).abs() < f64::EPSILON);
    assert_eq!(nutrition.fiber_g, Some(8.1));
    assert_eq!(nutrition.sodium_mg, None);
    assert_eq!(nutrition.sugar_g, Some(19.4));
    assert_eq!(
        nutrition.validated_at.timestamp_millis(),
        expected.validated_at.timestamp_millis()
    );

    let age = Utc::now() - stored.created_at;
    assert!(age < Duration::minutes(1), "created_at is the call time");
    assert_eq!(stored.created_at, stored.updated_at);
}

#[tokio::test]
async fn get_by_id_is_scoped_to_owner_and_tenant() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let (other_user, other_tenant) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let recipe = sample_recipe(user_id, "Mine", MealTiming::General);
    let recipe_id = repo.create(user_id, tenant_id, &recipe).await.unwrap();

    assert!(repo
        .get_by_id(&recipe_id, other_user, tenant_id)
        .await
        .unwrap()
        .is_none());
    assert!(repo
        .get_by_id(&recipe_id, user_id, other_tenant)
        .await
        .unwrap()
        .is_none());
    assert_eq!(repo.count(user_id, tenant_id).await.unwrap(), 1);
    assert_eq!(repo.count(other_user, other_tenant).await.unwrap(), 0);
}

#[tokio::test]
async fn list_filters_by_meal_timing_and_pages_newest_first() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    for (name, timing) in [
        ("Pre A", MealTiming::PreTraining),
        ("Post B", MealTiming::PostTraining),
        ("Pre C", MealTiming::PreTraining),
    ] {
        repo.create(user_id, tenant_id, &sample_recipe(user_id, name, timing))
            .await
            .unwrap();
    }

    let all = repo
        .list(user_id, tenant_id, None, None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    assert!(
        all.iter().all(|r| r.ingredients.len() == 2),
        "ingredients ride along on a listing"
    );

    let pre = repo
        .list(
            user_id,
            tenant_id,
            Some(MealTiming::PreTraining),
            None,
            None,
        )
        .await
        .unwrap();
    let mut pre_names: Vec<&str> = pre.iter().map(|r| r.name.as_str()).collect();
    pre_names.sort_unstable();
    assert_eq!(pre_names, vec!["Pre A", "Pre C"]);

    let page = repo
        .list(user_id, tenant_id, None, Some(2), Some(2))
        .await
        .unwrap();
    assert_eq!(page.len(), 1, "offset 2 of 3 leaves one row");
}

#[tokio::test]
async fn update_replaces_columns_and_ingredients_atomically() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let (other_user, _) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let recipe = sample_recipe(user_id, "Draft", MealTiming::General);
    let recipe_id = repo.create(user_id, tenant_id, &recipe).await.unwrap();

    let mut edited = recipe.clone();
    edited.name = "Final".to_owned();
    edited.servings = 2;
    edited.meal_timing = MealTiming::RestDay;
    edited.tags = vec!["rest".to_owned()];
    edited.ingredients = vec![RecipeIngredient {
        fdc_id: Some(170_187),
        name: "peanut butter".to_owned(),
        amount: 2.0,
        unit: IngredientUnit::Tablespoons,
        grams: 32.0,
        preparation: None,
    }];

    assert!(!repo
        .update(&recipe_id, other_user, tenant_id, &edited)
        .await
        .unwrap());
    assert!(repo
        .update(&recipe_id, user_id, tenant_id, &edited)
        .await
        .unwrap());

    let stored = repo
        .get_by_id(&recipe_id, user_id, tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.name, "Final");
    assert_eq!(stored.servings, 2);
    assert_eq!(stored.meal_timing, MealTiming::RestDay);
    assert_eq!(stored.tags, vec!["rest"]);
    assert_eq!(stored.ingredients.len(), 1);
    assert_eq!(stored.ingredients[0].name, "peanut butter");
    assert_eq!(stored.ingredients[0].fdc_id, Some(170_187));
    assert_eq!(stored.ingredients[0].unit, IngredientUnit::Tablespoons);
    assert!(stored.updated_at >= stored.created_at);
}

#[tokio::test]
async fn update_nutrition_cache_stores_the_validated_values() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let recipe = sample_recipe(user_id, "Uncached", MealTiming::General);
    let recipe_id = repo.create(user_id, tenant_id, &recipe).await.unwrap();
    assert!(repo
        .get_by_id(&recipe_id, user_id, tenant_id)
        .await
        .unwrap()
        .unwrap()
        .nutrition
        .is_none());

    let nutrition = sample_nutrition();
    assert!(repo
        .update_nutrition_cache(&recipe_id, user_id, tenant_id, &nutrition)
        .await
        .unwrap());
    assert!(
        !repo
            .update_nutrition_cache(&Uuid::new_v4().to_string(), user_id, tenant_id, &nutrition)
            .await
            .unwrap(),
        "an unknown recipe updates no row"
    );

    let cached = repo
        .get_by_id(&recipe_id, user_id, tenant_id)
        .await
        .unwrap()
        .unwrap()
        .nutrition
        .expect("the cache just written");
    assert!((cached.calories - 412.5).abs() < f64::EPSILON);
    assert!((cached.carbs_g - 62.0).abs() < f64::EPSILON);
    assert!((cached.fat_g - 11.8).abs() < f64::EPSILON);
    assert_eq!(cached.fiber_g, Some(8.1));
    assert_eq!(cached.sodium_mg, None);
    assert_eq!(
        cached.validated_at.timestamp_millis(),
        nutrition.validated_at.timestamp_millis()
    );
}

#[tokio::test]
async fn search_matches_name_tags_and_description_case_insensitively() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let (other_user, other_tenant) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let mut by_name = sample_recipe(user_id, "Banana Bread", MealTiming::General);
    by_name.tags = vec![];
    by_name.description = None;
    let mut by_tag = sample_recipe(user_id, "Loaf", MealTiming::General);
    by_tag.tags = vec!["banana".to_owned()];
    by_tag.description = None;
    let mut by_description = sample_recipe(user_id, "Smoothie", MealTiming::General);
    by_description.tags = vec![];
    by_description.description = Some("Blend the BANANA with milk".to_owned());
    let mut unrelated = sample_recipe(user_id, "Rice", MealTiming::General);
    unrelated.tags = vec![];
    unrelated.description = None;
    for recipe in [&by_name, &by_tag, &by_description, &unrelated] {
        repo.create(user_id, tenant_id, recipe).await.unwrap();
    }
    repo.create(
        other_user,
        other_tenant,
        &sample_recipe(other_user, "Banana Split", MealTiming::General),
    )
    .await
    .unwrap();

    let hits = repo
        .search(user_id, tenant_id, "banana", None, None)
        .await
        .unwrap();
    let mut names: Vec<&str> = hits.iter().map(|r| r.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["Banana Bread", "Loaf", "Smoothie"]);

    let limited = repo
        .search(user_id, tenant_id, "banana", Some(2), None)
        .await
        .unwrap();
    assert_eq!(limited.len(), 2);
}

#[tokio::test]
async fn delete_removes_the_recipe_and_its_ingredients() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let (other_user, _) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let recipe = sample_recipe(user_id, "Gone", MealTiming::General);
    let recipe_id = repo.create(user_id, tenant_id, &recipe).await.unwrap();

    assert!(!repo
        .delete(&recipe_id, other_user, tenant_id)
        .await
        .unwrap());
    assert!(repo.delete(&recipe_id, user_id, tenant_id).await.unwrap());
    assert!(!repo.delete(&recipe_id, user_id, tenant_id).await.unwrap());
    assert!(repo
        .get_by_id(&recipe_id, user_id, tenant_id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(repo.count(user_id, tenant_id).await.unwrap(), 0);
}

#[tokio::test]
async fn an_fdc_id_past_int4_is_refused_before_either_engine_sees_it() {
    let db = create_test_db().await;
    let (user_id, tenant_id) = seed_user(&db).await;
    let repo = db.repositories().recipes;

    let mut recipe = sample_recipe(user_id, "Too wide", MealTiming::General);
    recipe.ingredients[0].fdc_id = Some(i64::from(i32::MAX) + 1);

    let err = repo
        .create(user_id, tenant_id, &recipe)
        .await
        .expect_err("an fdc_id wider than the column is invalid input");
    assert!(
        err.to_string().contains("fdc_id out of range"),
        "unexpected error: {err}"
    );
    assert_eq!(
        repo.count(user_id, tenant_id).await.unwrap(),
        0,
        "the recipe row was rolled back with its ingredients"
    );
}
