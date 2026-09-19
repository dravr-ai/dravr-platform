// ABOUTME: Repository trait for tenant-scoped recipe persistence with USDA nutrition caching
// ABOUTME: Every statement written once as a shared const, one generic row parser, one macro emitting each backend's impl
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::fmt::Display;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::recipes::{
    IngredientUnit, MealTiming, Recipe, RecipeIngredient, ValidatedNutrition,
};
use pierre_core::models::TenantId;
use sqlx::{ColumnIndex, Decode, Row, Type};
use uuid::Uuid;

/// Recipe storage and management repository (tenant-scoped)
#[async_trait]
pub trait RecipeRepository: Send + Sync {
    /// Create a new recipe
    async fn create(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<String>;
    /// Get recipe by ID
    async fn get_by_id(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Recipe>>;
    /// List recipes with optional filtering
    async fn list(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        meal_timing: Option<MealTiming>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>>;
    /// Update an existing recipe
    async fn update(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        recipe: &Recipe,
    ) -> AppResult<bool>;
    /// Delete a recipe
    async fn delete(&self, recipe_id: &str, user_id: Uuid, tenant_id: TenantId) -> AppResult<bool>;
    /// Update cached nutrition data for a recipe
    async fn update_nutrition_cache(
        &self,
        recipe_id: &str,
        user_id: Uuid,
        tenant_id: TenantId,
        nutrition: &ValidatedNutrition,
    ) -> AppResult<bool>;
    /// Search recipes by text query
    async fn search(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        query: &str,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<Recipe>>;
    /// Count recipes
    async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32>;
}

// ============================================================================
// Statements
// ============================================================================
//
// `$n` placeholders throughout: sqlx accepts them on `SQLite` as well as
// Postgres, so one statement serves both backends and cannot drift between
// them. `user_id` binds through the backend's uuid codec (see
// [`super::uuid_columns`]): `uuid` on Postgres, hyphenated `TEXT` on `SQLite`.
// `tenant_id` is `TEXT` on both schemas and binds as its string on both.
// Timestamps bind as `DateTime<Utc>` on both: sqlx-sqlite writes the RFC 3339
// text the column already holds, Postgres its native `TIMESTAMPTZ`.

/// The twenty-one columns every read of `recipes` returns, in the order
/// [`recipe_from_row`] reads them, and the order `INSERT` names them.
macro_rules! recipe_columns {
    () => {
        "id, user_id, tenant_id, name, description, servings, \
         prep_time_mins, cook_time_mins, instructions, tags, meal_timing, \
         cached_calories, cached_protein_g, cached_carbs_g, cached_fat_g, \
         cached_fiber_g, cached_sodium_mg, cached_sugar_g, nutrition_validated_at, \
         created_at, updated_at"
    };
}
pub(crate) use recipe_columns;

/// The nine columns of `recipe_ingredients`, in the order [`ingredient_from_row`]
/// reads them and `INSERT` names them.
macro_rules! ingredient_columns {
    () => {
        "id, recipe_id, fdc_id, name, amount, unit, grams, preparation, sort_order"
    };
}

/// Insert a recipe; `$20` is the call time, bound to both timestamps.
pub(crate) const INSERT_RECIPE_SQL: &str = concat!(
    "INSERT INTO recipes (",
    recipe_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, \
     $12, $13, $14, $15, $16, $17, $18, $19, $20, $20)"
);

/// Insert one ingredient of a recipe.
pub(crate) const INSERT_INGREDIENT_SQL: &str = concat!(
    "INSERT INTO recipe_ingredients (",
    ingredient_columns!(),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
);

/// One recipe, owner-scoped.
pub(crate) const GET_RECIPE_SQL: &str = concat!(
    "SELECT ",
    recipe_columns!(),
    " FROM recipes WHERE id = $1 AND user_id = $2 AND tenant_id = $3"
);

/// A page of the owner's recipes, newest change first.
pub(crate) const LIST_RECIPES_SQL: &str = concat!(
    "SELECT ",
    recipe_columns!(),
    " FROM recipes WHERE user_id = $1 AND tenant_id = $2 \
     ORDER BY updated_at DESC LIMIT $3 OFFSET $4"
);

/// A page of the owner's recipes for one meal timing, newest change first.
pub(crate) const LIST_RECIPES_BY_TIMING_SQL: &str = concat!(
    "SELECT ",
    recipe_columns!(),
    " FROM recipes WHERE user_id = $1 AND tenant_id = $2 AND meal_timing = $3 \
     ORDER BY updated_at DESC LIMIT $4 OFFSET $5"
);

/// Replace every editable column of an owner's recipe.
pub(crate) const UPDATE_RECIPE_SQL: &str = "UPDATE recipes SET \
     name = $1, description = $2, servings = $3, \
     prep_time_mins = $4, cook_time_mins = $5, \
     instructions = $6, tags = $7, meal_timing = $8, \
     cached_calories = $9, cached_protein_g = $10, cached_carbs_g = $11, \
     cached_fat_g = $12, cached_fiber_g = $13, cached_sodium_mg = $14, \
     cached_sugar_g = $15, nutrition_validated_at = $16, updated_at = $17 \
     WHERE id = $18 AND user_id = $19 AND tenant_id = $20";

/// Drop a recipe's ingredients ahead of re-inserting the edited list.
pub(crate) const DELETE_INGREDIENTS_SQL: &str =
    "DELETE FROM recipe_ingredients WHERE recipe_id = $1";

/// Delete an owner's recipe; its ingredients go by `ON DELETE CASCADE`.
pub(crate) const DELETE_RECIPE_SQL: &str =
    "DELETE FROM recipes WHERE id = $1 AND user_id = $2 AND tenant_id = $3";

/// Store validated nutrition on an owner's recipe.
pub(crate) const UPDATE_NUTRITION_CACHE_SQL: &str = "UPDATE recipes SET \
     cached_calories = $1, cached_protein_g = $2, cached_carbs_g = $3, \
     cached_fat_g = $4, cached_fiber_g = $5, cached_sodium_mg = $6, \
     cached_sugar_g = $7, nutrition_validated_at = $8, updated_at = $9 \
     WHERE id = $10 AND user_id = $11 AND tenant_id = $12";

/// A page of the owner's recipes whose name, tags or description contains
/// the query. `$like` is the backend's case-folding match operator.
macro_rules! search_recipes_sql {
    ($like:literal) => {
        concat!(
            "SELECT ",
            recipe_columns!(),
            " FROM recipes WHERE user_id = $1 AND tenant_id = $2 AND (name ",
            $like,
            " $3 OR tags ",
            $like,
            " $3 OR description ",
            $like,
            " $3) ORDER BY updated_at DESC LIMIT $4 OFFSET $5"
        )
    };
}
pub(crate) use search_recipes_sql;

/// How many recipes the owner has.
pub(crate) const COUNT_RECIPES_SQL: &str =
    "SELECT COUNT(*) AS count FROM recipes WHERE user_id = $1 AND tenant_id = $2";

/// One recipe's ingredients, in display order.
pub(crate) const GET_INGREDIENTS_SQL: &str = concat!(
    "SELECT ",
    ingredient_columns!(),
    " FROM recipe_ingredients WHERE recipe_id = $1 ORDER BY sort_order"
);

/// The ingredients of `count` recipes in one statement, `$1..$count` the
/// recipe ids, grouped by recipe then display order. Two queries per listing
/// instead of one per recipe.
pub(crate) fn ingredients_batch_sql(count: usize) -> String {
    let placeholders: Vec<String> = (1..=count).map(|i| format!("${i}")).collect();
    format!(
        "SELECT {} FROM recipe_ingredients WHERE recipe_id IN ({}) ORDER BY recipe_id, sort_order",
        ingredient_columns!(),
        placeholders.join(", ")
    )
}

/// The `%query%` pattern for a free-text search.
pub(crate) fn contains_pattern(query: &str) -> String {
    format!("%{query}%")
}

/// A page bound a caller left unset or out of `i32` range falls to the
/// method's default.
pub(crate) fn page_bound(value: Option<u32>, default: i32) -> i32 {
    value.and_then(|v| i32::try_from(v).ok()).unwrap_or(default)
}

/// The USDA `FoodData` Central id as the `int4` the column holds.
///
/// The domain carries it as `i64`; `recipe_ingredients.fdc_id` is `INTEGER`,
/// which is `int4` on Postgres (a wider value is a driver error) and 64-bit
/// on `SQLite` (a wider value is stored, then truncated on the `i32` read).
/// One check at the bind refuses it as invalid input on both engines.
///
/// # Errors
/// Returns `AppError::invalid_input` when the id does not fit the column.
pub(crate) fn fdc_id_column(fdc_id: Option<i64>) -> AppResult<Option<i32>> {
    fdc_id
        .map(i32::try_from)
        .transpose()
        .map_err(|_| AppError::invalid_input("fdc_id out of range"))
}

/// The position of an ingredient in its recipe, as the column holds it.
///
/// # Errors
/// Returns `AppError::invalid_input` for a list longer than the column can
/// number.
pub(crate) fn sort_order_column(index: usize) -> AppResult<i32> {
    i32::try_from(index).map_err(|_| AppError::invalid_input("too many ingredients"))
}

// ============================================================================
// Column codecs
// ============================================================================

pub(crate) const fn meal_timing_to_string(timing: MealTiming) -> &'static str {
    match timing {
        MealTiming::PreTraining => "pre_training",
        MealTiming::PostTraining => "post_training",
        MealTiming::RestDay => "rest_day",
        MealTiming::General => "general",
    }
}

pub(crate) fn string_to_meal_timing(s: &str) -> MealTiming {
    match s {
        "pre_training" => MealTiming::PreTraining,
        "post_training" => MealTiming::PostTraining,
        "rest_day" => MealTiming::RestDay,
        _ => MealTiming::General,
    }
}

pub(crate) const fn unit_to_string(unit: IngredientUnit) -> &'static str {
    match unit {
        IngredientUnit::Grams => "grams",
        IngredientUnit::Milliliters => "milliliters",
        IngredientUnit::Cups => "cups",
        IngredientUnit::Tablespoons => "tablespoons",
        IngredientUnit::Teaspoons => "teaspoons",
        IngredientUnit::Pieces => "pieces",
        IngredientUnit::Ounces => "ounces",
        IngredientUnit::Pounds => "pounds",
        IngredientUnit::Kilograms => "kilograms",
    }
}

/// Grams for an unknown spelling, including "grams" itself: the schema's
/// `CHECK` keeps the stored set to the nine names above.
pub(crate) fn string_to_unit(s: &str) -> IngredientUnit {
    match s {
        "milliliters" => IngredientUnit::Milliliters,
        "cups" => IngredientUnit::Cups,
        "tablespoons" => IngredientUnit::Tablespoons,
        "teaspoons" => IngredientUnit::Teaspoons,
        "pieces" => IngredientUnit::Pieces,
        "ounces" => IngredientUnit::Ounces,
        "pounds" => IngredientUnit::Pounds,
        "kilograms" => IngredientUnit::Kilograms,
        _ => IngredientUnit::Grams,
    }
}

// ============================================================================
// Row parsers
// ============================================================================

fn recipe_column_error(name: &str, e: impl Display) -> AppError {
    AppError::database(format!("Failed to read recipe column {name}: {e}"))
}

/// Read one column, naming it in the error.
fn column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    T: Decode<'r, R::Database> + Type<R::Database>,
{
    row.try_get(name).map_err(|e| recipe_column_error(name, e))
}

/// A NOT NULL `INTEGER` column the domain holds as a narrower unsigned type.
fn narrow_column<'r, R, T>(row: &'r R, name: &str) -> AppResult<T>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    T: TryFrom<i32>,
    T::Error: Display,
{
    let raw: i32 = column(row, name)?;
    T::try_from(raw).map_err(|e| recipe_column_error(name, e))
}

/// A nullable `INTEGER` column the domain holds as a narrower unsigned type.
fn narrow_column_opt<'r, R, T>(row: &'r R, name: &str) -> AppResult<Option<T>>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    T: TryFrom<i32>,
    T::Error: Display,
{
    let raw: Option<i32> = column(row, name)?;
    raw.map(|v| T::try_from(v).map_err(|e| recipe_column_error(name, e)))
        .transpose()
}

/// Convert a `recipes` row to a [`Recipe`].
///
/// `user_id` is read by the caller through its backend's uuid codec and
/// handed in; everything else decodes alike on both drivers: `id` as the
/// hyphenated text both schemas store, timestamps as `DateTime<Utc>`,
/// counts as `i32`.
///
/// The nutrition cache is present when `cached_calories` is; a cached row
/// whose `nutrition_validated_at` is NULL reports the read time as its
/// validation time, on both backends.
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn recipe_from_row<'r, R>(
    row: &'r R,
    user_id: Uuid,
    ingredients: Vec<RecipeIngredient>,
) -> AppResult<Recipe>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    f64: Decode<'r, R::Database> + Type<R::Database>,
    DateTime<Utc>: Decode<'r, R::Database> + Type<R::Database>,
{
    let id: String = column(row, "id")?;
    let meal_timing: String = column(row, "meal_timing")?;
    let instructions: String = column(row, "instructions")?;
    let tags: String = column(row, "tags")?;

    let cached_calories: Option<f64> = column(row, "cached_calories")?;
    let nutrition = match cached_calories {
        Some(calories) => {
            let validated_at: Option<DateTime<Utc>> = column(row, "nutrition_validated_at")?;
            Some(ValidatedNutrition {
                calories,
                protein_g: column::<_, Option<f64>>(row, "cached_protein_g")?.unwrap_or(0.0),
                carbs_g: column::<_, Option<f64>>(row, "cached_carbs_g")?.unwrap_or(0.0),
                fat_g: column::<_, Option<f64>>(row, "cached_fat_g")?.unwrap_or(0.0),
                fiber_g: column(row, "cached_fiber_g")?,
                sodium_mg: column(row, "cached_sodium_mg")?,
                sugar_g: column(row, "cached_sugar_g")?,
                validated_at: validated_at.unwrap_or_else(Utc::now),
            })
        }
        None => None,
    };

    Ok(Recipe {
        id: Uuid::parse_str(&id).map_err(|e| recipe_column_error("id", e))?,
        user_id,
        name: column(row, "name")?,
        description: column(row, "description")?,
        servings: narrow_column(row, "servings")?,
        prep_time_mins: narrow_column_opt(row, "prep_time_mins")?,
        cook_time_mins: narrow_column_opt(row, "cook_time_mins")?,
        ingredients,
        instructions: serde_json::from_str(&instructions)
            .map_err(|e| recipe_column_error("instructions", e))?,
        tags: serde_json::from_str(&tags).map_err(|e| recipe_column_error("tags", e))?,
        nutrition,
        meal_timing: string_to_meal_timing(&meal_timing),
        created_at: column(row, "created_at")?,
        updated_at: column(row, "updated_at")?,
    })
}

/// Convert a `recipe_ingredients` row to a [`RecipeIngredient`]. `fdc_id`
/// is the column's `int4`, widened to the domain's `i64`.
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn ingredient_from_row<'r, R>(row: &'r R) -> AppResult<RecipeIngredient>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    f64: Decode<'r, R::Database> + Type<R::Database>,
{
    let unit: String = column(row, "unit")?;
    let fdc_id: Option<i32> = column(row, "fdc_id")?;
    Ok(RecipeIngredient {
        fdc_id: fdc_id.map(i64::from),
        name: column(row, "name")?,
        amount: column(row, "amount")?,
        unit: string_to_unit(&unit),
        grams: column(row, "grams")?,
        preparation: column(row, "preparation")?,
    })
}

/// Group a batch of ingredient rows by their recipe id, in row order.
///
/// # Errors
/// Returns a database error naming the column that would not decode.
pub(crate) fn ingredients_by_recipe<'r, R>(
    rows: &'r [R],
) -> AppResult<HashMap<String, Vec<RecipeIngredient>>>
where
    R: Row,
    for<'a> &'a str: ColumnIndex<R>,
    String: Decode<'r, R::Database> + Type<R::Database>,
    i32: Decode<'r, R::Database> + Type<R::Database>,
    f64: Decode<'r, R::Database> + Type<R::Database>,
{
    let mut by_recipe: HashMap<String, Vec<RecipeIngredient>> = HashMap::new();
    for row in rows {
        let recipe_id: String = column(row, "recipe_id")?;
        by_recipe
            .entry(recipe_id)
            .or_default()
            .push(ingredient_from_row(row)?);
    }
    Ok(by_recipe)
}

/// Insert a recipe's ingredients through an open transaction guard; each
/// row's failure is reported under `$failure`. An expression, so the
/// driver is the one the guard's transaction already carries.
macro_rules! insert_recipe_ingredients {
    ($guard:expr, $recipe_id:expr, $ingredients:expr, $failure:literal) => {{
        for (index, ingredient) in $ingredients.iter().enumerate() {
            sqlx::query(INSERT_INGREDIENT_SQL)
                .bind(Uuid::new_v4().to_string())
                .bind($recipe_id)
                .bind(fdc_id_column(ingredient.fdc_id)?)
                .bind(&ingredient.name)
                .bind(ingredient.amount)
                .bind(unit_to_string(ingredient.unit))
                .bind(ingredient.grams)
                .bind(&ingredient.preparation)
                .bind(sort_order_column(index)?)
                .execute($guard.executor()?)
                .await
                .map_err(|e| AppError::database(format!("{}: {e}", $failure)))?;
        }
    }};
}
pub(crate) use insert_recipe_ingredients;

/// A page of recipe rows with their ingredients attached, in two queries.
/// An expression, so the row type is the one `$repo.pool()` returned and
/// `$ids::read` can decode `user_id` from it.
macro_rules! recipes_with_ingredients {
    ($repo:expr, $rows:expr, $ids:ident) => {{
        let rows = $rows;
        let recipe_ids = rows
            .iter()
            .map(|row| row.try_get::<String, _>("id"))
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| AppError::database(format!("Failed to read recipe id: {e}")))?;
        let mut ingredients = $repo.recipe_ingredients_batch(&recipe_ids).await?;
        let mut recipes = Vec::with_capacity(rows.len());
        for (row, recipe_id) in rows.iter().zip(recipe_ids) {
            let user_id = $ids::read(row, "user_id")?;
            let recipe_ingredients = ingredients.remove(&recipe_id).unwrap_or_default();
            recipes.push(recipe_from_row(row, user_id, recipe_ingredients)?);
        }
        Ok(recipes)
    }};
}
pub(crate) use recipes_with_ingredients;

// ============================================================================
// The implementation, emitted once per backend
// ============================================================================

/// Emit the whole [`RecipeRepository`] implementation for one backend type.
/// The body is written once here; each backend's shell invokes it with its
/// own type, its uuid codec and its match operator, and sqlx resolves the
/// driver from `self.pool()` per expansion.
///
/// `$ids` is the codec in [`super::uuid_columns`] for how that backend's
/// `user_id` column binds and reads. `$like` is the backend's case-folding
/// text match, spliced into the search: `"ILIKE"` on Postgres, `"LIKE"` on
/// `SQLite`. `SQLite`'s `LIKE` folds case for ASCII letters only, so a search
/// for a name with an accented letter is case-sensitive there and not on
/// Postgres; production runs Postgres, so no athlete sees the narrower fold.
///
/// `create` and `update` each run in one transaction through
/// [`TransactionGuard`](crate::backends::shared::transactions::TransactionGuard):
/// a failed ingredient insert rolls the recipe back with it.
macro_rules! impl_recipe_repository {
    ($ty:ty, $ids:ident, $like:literal) => {
        impl $ty {
            /// The ingredients of one recipe, in display order.
            async fn recipe_ingredients(
                &self,
                recipe_id: &str,
            ) -> AppResult<Vec<RecipeIngredient>> {
                let rows = sqlx::query(GET_INGREDIENTS_SQL)
                    .bind(recipe_id)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to get recipe ingredients: {e}"))
                    })?;
                rows.iter().map(ingredient_from_row).collect()
            }

            /// The ingredients of a page of recipes, grouped by recipe id.
            async fn recipe_ingredients_batch(
                &self,
                recipe_ids: &[String],
            ) -> AppResult<HashMap<String, Vec<RecipeIngredient>>> {
                if recipe_ids.is_empty() {
                    return Ok(HashMap::new());
                }
                let sql = ingredients_batch_sql(recipe_ids.len());
                let mut query = sqlx::query(&sql);
                for recipe_id in recipe_ids {
                    query = query.bind(recipe_id);
                }
                let rows = query.fetch_all(self.pool()).await.map_err(|e| {
                    AppError::database(format!("Failed to batch fetch ingredients: {e}"))
                })?;
                ingredients_by_recipe(&rows)
            }
        }

        #[async_trait::async_trait]
        impl RecipeRepository for $ty {
            async fn create(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                recipe: &Recipe,
            ) -> AppResult<String> {
                let now = Utc::now();
                let recipe_id = recipe.id.to_string();
                let instructions_json = serde_json::to_string(&recipe.instructions)?;
                let tags_json = serde_json::to_string(&recipe.tags)?;

                let tx =
                    self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("Failed to begin transaction: {e}"))
                    })?;
                let mut guard = TransactionGuard::new(tx);

                sqlx::query(INSERT_RECIPE_SQL)
                    .bind(&recipe_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .bind(&recipe.name)
                    .bind(&recipe.description)
                    .bind(i32::from(recipe.servings))
                    .bind(recipe.prep_time_mins.map(i32::from))
                    .bind(recipe.cook_time_mins.map(i32::from))
                    .bind(&instructions_json)
                    .bind(&tags_json)
                    .bind(meal_timing_to_string(recipe.meal_timing))
                    .bind(recipe.nutrition.as_ref().map(|n| n.calories))
                    .bind(recipe.nutrition.as_ref().map(|n| n.protein_g))
                    .bind(recipe.nutrition.as_ref().map(|n| n.carbs_g))
                    .bind(recipe.nutrition.as_ref().map(|n| n.fat_g))
                    .bind(recipe.nutrition.as_ref().and_then(|n| n.fiber_g))
                    .bind(recipe.nutrition.as_ref().and_then(|n| n.sodium_mg))
                    .bind(recipe.nutrition.as_ref().and_then(|n| n.sugar_g))
                    .bind(recipe.nutrition.as_ref().map(|n| n.validated_at))
                    .bind(now)
                    .execute(guard.executor()?)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to create recipe: {e}")))?;

                insert_recipe_ingredients!(
                    guard,
                    &recipe_id,
                    recipe.ingredients,
                    "Failed to create recipe ingredient"
                );

                guard.commit().await?;
                Ok(recipe_id)
            }

            async fn get_by_id(
                &self,
                recipe_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<Option<Recipe>> {
                let row = sqlx::query(GET_RECIPE_SQL)
                    .bind(recipe_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to get recipe: {e}")))?;
                let Some(row) = row else { return Ok(None) };
                let owner = $ids::read(&row, "user_id")?;
                let ingredients = self.recipe_ingredients(recipe_id).await?;
                Ok(Some(recipe_from_row(&row, owner, ingredients)?))
            }

            async fn list(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                meal_timing: Option<MealTiming>,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<Recipe>> {
                let limit = page_bound(limit, 50);
                let offset = page_bound(offset, 0);
                let rows = match meal_timing {
                    Some(timing) => {
                        sqlx::query(LIST_RECIPES_BY_TIMING_SQL)
                            .bind($ids::bind(user_id))
                            .bind(tenant_id.to_string())
                            .bind(meal_timing_to_string(timing))
                            .bind(limit)
                            .bind(offset)
                            .fetch_all(self.pool())
                            .await
                    }
                    None => {
                        sqlx::query(LIST_RECIPES_SQL)
                            .bind($ids::bind(user_id))
                            .bind(tenant_id.to_string())
                            .bind(limit)
                            .bind(offset)
                            .fetch_all(self.pool())
                            .await
                    }
                }
                .map_err(|e| AppError::database(format!("Failed to list recipes: {e}")))?;
                recipes_with_ingredients!(self, rows, $ids)
            }

            async fn update(
                &self,
                recipe_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
                recipe: &Recipe,
            ) -> AppResult<bool> {
                let now = Utc::now();
                let instructions_json = serde_json::to_string(&recipe.instructions)?;
                let tags_json = serde_json::to_string(&recipe.tags)?;

                let tx =
                    self.pool().begin().await.map_err(|e| {
                        AppError::database(format!("Failed to begin transaction: {e}"))
                    })?;
                let mut guard = TransactionGuard::new(tx);

                let result = sqlx::query(UPDATE_RECIPE_SQL)
                    .bind(&recipe.name)
                    .bind(&recipe.description)
                    .bind(i32::from(recipe.servings))
                    .bind(recipe.prep_time_mins.map(i32::from))
                    .bind(recipe.cook_time_mins.map(i32::from))
                    .bind(&instructions_json)
                    .bind(&tags_json)
                    .bind(meal_timing_to_string(recipe.meal_timing))
                    .bind(recipe.nutrition.as_ref().map(|n| n.calories))
                    .bind(recipe.nutrition.as_ref().map(|n| n.protein_g))
                    .bind(recipe.nutrition.as_ref().map(|n| n.carbs_g))
                    .bind(recipe.nutrition.as_ref().map(|n| n.fat_g))
                    .bind(recipe.nutrition.as_ref().and_then(|n| n.fiber_g))
                    .bind(recipe.nutrition.as_ref().and_then(|n| n.sodium_mg))
                    .bind(recipe.nutrition.as_ref().and_then(|n| n.sugar_g))
                    .bind(recipe.nutrition.as_ref().map(|n| n.validated_at))
                    .bind(now)
                    .bind(recipe_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .execute(guard.executor()?)
                    .await
                    .map_err(|e| AppError::database(format!("Failed to update recipe: {e}")))?;

                if result.rows_affected() == 0 {
                    // Not the caller's recipe: the guard rolls the open transaction back on drop.
                    return Ok(false);
                }

                sqlx::query(DELETE_INGREDIENTS_SQL)
                    .bind(recipe_id)
                    .execute(guard.executor()?)
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to delete recipe ingredients: {e}"))
                    })?;

                insert_recipe_ingredients!(
                    guard,
                    recipe_id,
                    recipe.ingredients,
                    "Failed to update recipe ingredient"
                );

                guard.commit().await?;
                Ok(true)
            }

            async fn delete(
                &self,
                recipe_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
            ) -> AppResult<bool> {
                let result = sqlx::query(DELETE_RECIPE_SQL)
                    .bind(recipe_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to delete recipe: {e}")))?;
                Ok(result.rows_affected() > 0)
            }

            async fn update_nutrition_cache(
                &self,
                recipe_id: &str,
                user_id: Uuid,
                tenant_id: TenantId,
                nutrition: &ValidatedNutrition,
            ) -> AppResult<bool> {
                let result = sqlx::query(UPDATE_NUTRITION_CACHE_SQL)
                    .bind(nutrition.calories)
                    .bind(nutrition.protein_g)
                    .bind(nutrition.carbs_g)
                    .bind(nutrition.fat_g)
                    .bind(nutrition.fiber_g)
                    .bind(nutrition.sodium_mg)
                    .bind(nutrition.sugar_g)
                    .bind(nutrition.validated_at)
                    .bind(Utc::now())
                    .bind(recipe_id)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .execute(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to update nutrition cache: {e}"))
                    })?;
                Ok(result.rows_affected() > 0)
            }

            async fn search(
                &self,
                user_id: Uuid,
                tenant_id: TenantId,
                query: &str,
                limit: Option<u32>,
                offset: Option<u32>,
            ) -> AppResult<Vec<Recipe>> {
                let rows = sqlx::query(search_recipes_sql!($like))
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .bind(contains_pattern(query))
                    .bind(page_bound(limit, 20))
                    .bind(page_bound(offset, 0))
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to search recipes: {e}")))?;
                recipes_with_ingredients!(self, rows, $ids)
            }

            async fn count(&self, user_id: Uuid, tenant_id: TenantId) -> AppResult<u32> {
                let row = sqlx::query(COUNT_RECIPES_SQL)
                    .bind($ids::bind(user_id))
                    .bind(tenant_id.to_string())
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Failed to count recipes: {e}")))?;
                let count: i64 = row
                    .try_get("count")
                    .map_err(|e| AppError::database(format!("Failed to read recipe count: {e}")))?;
                u32::try_from(count)
                    .map_err(|e| AppError::database(format!("Recipe count out of range: {e}")))
            }
        }
    };
}
pub(crate) use impl_recipe_repository;
