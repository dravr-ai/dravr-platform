// ABOUTME: Fuel-product tools — search the catalogue, and add, list and remove the gels, drink mixes and bars an athlete carries
// ABOUTME: Label facts reach plan arithmetic only once an operator approves them; an athlete's own entry is filed for review (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fuel-product tools
//!
//! The fuel-product bank (carnet#155) is a global catalogue of fuel products
//! with versioned label facts, and each athlete's list of the products they
//! carry. Four chat-callable tools in the `nutrition` category reach it:
//!
//! - `search_fuel_products` — catalogue products by words or barcode, each
//!   with its current per-serving facts and where each fact came from.
//! - `add_fuel_item` — carry a catalogue product, or file one the catalogue
//!   lacks from the facts the athlete reads off its label. That files a
//!   proposal an operator reviews: the item is pending, and its facts reach
//!   no plan until it is approved. The facts pass the plausibility gate
//!   first ([`check_plausibility`]): a label that cannot be what it states is
//!   refused with the violation codes, never adjusted.
//! - `list_fuel_items` — the athlete's items: approved with facts, pending, or
//!   rejected (shown as rejected, so the agent can say so).
//! - `remove_fuel_item` — stop carrying one.
//!
//! **Whose list.** Every call runs on the caller's own list, the way the
//! memory and nutrition tools do, and none takes an `athlete` argument. That
//! is what places them on a coach's seats (`crate::coach_seat`): in a group the
//! coach coaches all four are withheld, being data tools; in a coach's thread
//! about one athlete the two writes are withheld, so a product named there is
//! never filed on the coach's own list. The athlete's list reaches that
//! thread through the prompt's fuel-products block instead.
//!
//! **No medical-flag gate.** A label's facts describe a product, the same fact
//! `search_food` and `get_food_details` return, and set no amount for this
//! athlete; `nutrition_gate` gates prescriptions only. What a plan day
//! prescribes from these products is gated where the plan is saved.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    check_plausibility, fuel_natural_key, normalize_gtin, AthleteFuelItem, CatalogueFuelProduct,
    FactProvenance, FactSource, FieldProvenance, FuelFacts, FuelForm, FuelItemCarried,
    FuelItemEntry, NewFuelProposal, PlausibilityViolation, ProposalSource, ProposalStatus,
    TenantId,
};
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::annotations::{annotations, Effect, Reach};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};

/// Most catalogue products one search returns.
const MAX_SEARCH_RESULTS: i64 = 10;

/// Longest brand, name, variant or note an athlete may type.
const MAX_TEXT_LEN: usize = 120;

/// The forms an athlete may name, as stored.
const FORMS: &[&str] = &[
    "gel",
    "drink_mix",
    "bar",
    "chew",
    "capsule",
    "tablet",
    "other",
];

// ============================================================================
// Output shapes
// ============================================================================

/// One product's label facts per serving. Absent is what the label does not
/// state, never zero.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FuelLabelFacts {
    /// How the label names one serving ("1 gel (40 g)").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_label: Option<String>,
    /// Grams per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serving_g: Option<f64>,
    /// kcal per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_kcal: Option<f64>,
    /// Carbohydrate, g per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carbs_g: Option<f64>,
    /// Of which sugars, g per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sugars_g: Option<f64>,
    /// Protein, g per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protein_g: Option<f64>,
    /// Fat, g per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fat_g: Option<f64>,
    /// Sodium, mg per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sodium_mg: Option<f64>,
    /// Potassium, mg per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potassium_mg: Option<f64>,
    /// Caffeine, mg per serving.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caffeine_mg: Option<f64>,
    /// Fluid, ml per serving (a gel's volume, a drink mix's mixing volume).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fluid_ml: Option<f64>,
    /// Grams of fructose per gram of glucose source: 0.8 for 1:0.8, 0.5 for
    /// 2:1, 0 for glucose or maltodextrin only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glucose_fructose_ratio: Option<f64>,
}

impl FuelLabelFacts {
    fn from_facts(facts: &FuelFacts) -> Self {
        Self {
            serving_label: facts.serving_label.clone(),
            serving_g: facts.serving_g,
            energy_kcal: facts.energy_kcal,
            carbs_g: facts.carbs_g,
            sugars_g: facts.sugars_g,
            protein_g: facts.protein_g,
            fat_g: facts.fat_g,
            sodium_mg: facts.sodium_mg,
            potassium_mg: facts.potassium_mg,
            caffeine_mg: facts.caffeine_mg,
            fluid_ml: facts.fluid_ml,
            glucose_fructose_ratio: facts.glucose_fructose_ratio,
        }
    }

    fn to_facts(&self) -> FuelFacts {
        FuelFacts {
            serving_label: self.serving_label.clone(),
            serving_g: self.serving_g,
            energy_kcal: self.energy_kcal,
            carbs_g: self.carbs_g,
            sugars_g: self.sugars_g,
            protein_g: self.protein_g,
            fat_g: self.fat_g,
            sodium_mg: self.sodium_mg,
            potassium_mg: self.potassium_mg,
            caffeine_mg: self.caffeine_mg,
            fluid_ml: self.fluid_ml,
            glucose_fructose_ratio: self.glucose_fructose_ratio,
            ..FuelFacts::default()
        }
    }

    /// The stored names of the fields stated, for their provenance.
    fn stated_fields(&self) -> Vec<&'static str> {
        [
            ("serving_label", self.serving_label.is_some()),
            ("serving_g", self.serving_g.is_some()),
            ("energy_kcal", self.energy_kcal.is_some()),
            ("carbs_g", self.carbs_g.is_some()),
            ("sugars_g", self.sugars_g.is_some()),
            ("protein_g", self.protein_g.is_some()),
            ("fat_g", self.fat_g.is_some()),
            ("sodium_mg", self.sodium_mg.is_some()),
            ("potassium_mg", self.potassium_mg.is_some()),
            ("caffeine_mg", self.caffeine_mg.is_some()),
            ("fluid_ml", self.fluid_ml.is_some()),
            (
                "glucose_fructose_ratio",
                self.glucose_fructose_ratio.is_some(),
            ),
        ]
        .into_iter()
        .filter_map(|(name, stated)| stated.then_some(name))
        .collect()
    }
}

/// A catalogue product with its current, approved facts.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FuelProductView {
    /// The id `add_fuel_item` and a plan day's `fuel_product_ids` take.
    pub product_id: String,
    /// The brand.
    pub brand: String,
    /// The product name.
    pub name: String,
    /// The flavour or variant; empty when there is none.
    pub variant: String,
    /// gel, `drink_mix`, bar, chew, capsule, tablet or other.
    pub form: String,
    /// The barcode, GTIN-14, when on file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gtin: Option<String>,
    /// The label's facts per serving, as approved.
    pub facts: FuelLabelFacts,
    /// Where each fact came from: `label_ocr`, `athlete`, `admin_edit` or
    /// `brand_page`, by fact name.
    pub sources: BTreeMap<String, String>,
    /// When these facts became current, RFC 3339.
    pub facts_since: String,
}

impl FuelProductView {
    fn from_catalogue(product: &CatalogueFuelProduct) -> Self {
        Self {
            product_id: product.product.id.to_string(),
            brand: product.product.brand.clone(),
            name: product.product.name.clone(),
            variant: product.product.variant.clone(),
            form: product.product.form.as_str().to_owned(),
            gtin: product.product.gtin.clone(),
            facts: FuelLabelFacts::from_facts(&product.current.facts),
            sources: product
                .current
                .provenance
                .iter()
                .map(|(field, provenance)| (field.clone(), source_name(provenance.source)))
                .collect(),
            facts_since: product.current.valid_from.to_rfc3339(),
        }
    }
}

fn source_name(source: FactSource) -> String {
    match source {
        FactSource::LabelOcr => "label_ocr",
        FactSource::Athlete => "athlete",
        FactSource::AdminEdit => "admin_edit",
        FactSource::BrandPage => "brand_page",
    }
    .to_owned()
}

/// What `search_fuel_products` answers.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct SearchFuelProductsResult {
    /// Matching catalogue products, approved facts only.
    pub products: Vec<FuelProductView>,
    /// How many matched, at most ten.
    pub count: usize,
}

/// Where one carried item stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FuelItemStatus {
    /// A catalogue product with approved facts.
    Approved,
    /// Filed by the athlete, awaiting review: its facts are not used.
    Pending,
    /// Filed by the athlete and rejected at review: its facts are not used.
    Rejected,
}

/// One item the athlete carries.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct FuelItemView {
    /// The id `remove_fuel_item` takes.
    pub item_id: String,
    /// approved, pending or rejected.
    pub status: FuelItemStatus,
    /// The brand.
    pub brand: String,
    /// The product name.
    pub name: String,
    /// The variant; empty when there is none.
    pub variant: String,
    /// The form.
    pub form: String,
    /// The catalogue product, with its approved facts; absent until approved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<FuelProductView>,
    /// The athlete's note ("2 per hour after 90 min").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl FuelItemView {
    fn from_item(item: &AthleteFuelItem) -> Self {
        match &item.entry {
            FuelItemEntry::Catalogue(product) => Self {
                item_id: item.id.to_string(),
                status: FuelItemStatus::Approved,
                brand: product.product.brand.clone(),
                name: product.product.name.clone(),
                variant: product.product.variant.clone(),
                form: product.product.form.as_str().to_owned(),
                product: Some(FuelProductView::from_catalogue(product)),
                note: item.note.clone(),
            },
            FuelItemEntry::Pending(pending) => Self {
                item_id: item.id.to_string(),
                status: if pending.status == ProposalStatus::Rejected {
                    FuelItemStatus::Rejected
                } else {
                    FuelItemStatus::Pending
                },
                brand: pending.brand.clone(),
                name: pending.name.clone(),
                variant: pending.variant.clone(),
                form: pending.form.as_str().to_owned(),
                product: None,
                note: item.note.clone(),
            },
        }
    }
}

/// What `list_fuel_items` answers.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ListFuelItemsResult {
    /// The athlete's items, oldest first.
    pub items: Vec<FuelItemView>,
    /// How many.
    pub count: usize,
}

/// What `add_fuel_item` answers.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct AddFuelItemResult {
    /// The item, new or already carried.
    pub item: FuelItemView,
    /// True when the athlete already carried it; nothing was added.
    pub already_carried: bool,
    /// What to tell the athlete about the item's facts.
    pub message: String,
}

/// What `remove_fuel_item` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RemoveFuelItemResult {
    /// The item asked for.
    pub item_id: String,
    /// False when the athlete carried no such item.
    pub removed: bool,
}

// ============================================================================
// Shared helpers
// ============================================================================

fn string_property(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

fn number_property(description: &str) -> PropertySchema {
    PropertySchema {
        property_type: "number".to_owned(),
        description: Some(description.to_owned()),
        ..Default::default()
    }
}

fn optional_text(args: &Value, key: &str) -> AppResult<Option<String>> {
    let Some(text) = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return Ok(None);
    };
    if text.chars().count() > MAX_TEXT_LEN {
        return Err(AppError::invalid_input(format!(
            "{key} is longer than {MAX_TEXT_LEN} characters"
        )));
    }
    Ok(Some(text.to_owned()))
}

fn parse_id(args: &Value, key: &str) -> AppResult<Option<Uuid>> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|raw| {
            Uuid::parse_str(raw.trim())
                .map_err(|_| AppError::invalid_input(format!("{key} is not an id: {raw}")))
        })
        .transpose()
}

fn caller(context: &ToolExecutionContext) -> AppResult<(TenantId, Uuid)> {
    Ok((
        TenantId::from_uuid(context.require_tenant()?),
        context.user_id,
    ))
}

/// The athlete's item `id`, as listed.
async fn listed_item(
    context: &ToolExecutionContext,
    tenant: TenantId,
    user: Uuid,
    id: Uuid,
) -> AppResult<FuelItemView> {
    context
        .resources
        .repos()
        .athlete_fuel_items
        .list_fuel_items(tenant, user)
        .await?
        .iter()
        .find(|item| item.id == id)
        .map(FuelItemView::from_item)
        .ok_or_else(|| AppError::not_found(format!("Fuel item {id}")))
}

// ============================================================================
// search_fuel_products
// ============================================================================

/// Search the fuel-product catalogue.
#[derive(Debug)]
pub struct SearchFuelProductsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SearchFuelProductsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_owned(),
            string_property("Brand and product words ('maurten 160'), or a barcode."),
        );
        let schema = object_schema(properties, Some(vec!["query".to_owned()]));
        answers_with::<SearchFuelProductsResult>(tool_definition(
            "search_fuel_products",
            "Find a gel, drink mix, bar or chew in the fuel catalogue by name or barcode: approved label facts per serving (carbs, sodium, caffeine, glucose:fructose) with each fact's source.",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            caller(&context)?;
            let query = optional_text(&args, "query")?.ok_or_else(|| {
                AppError::invalid_input("query is required: brand and product words or a barcode")
            })?;
            let products = context
                .resources
                .repos()
                .fuel_catalogue
                .search_fuel_products(&query, MAX_SEARCH_RESULTS)
                .await?;
            let products: Vec<FuelProductView> = products
                .iter()
                .map(FuelProductView::from_catalogue)
                .collect();
            ok_typed(
                "search_fuel_products",
                SearchFuelProductsResult {
                    count: products.len(),
                    products,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// add_fuel_item
// ============================================================================

/// Carry a fuel product: from the catalogue, or filed for review.
#[derive(Debug)]
pub struct AddFuelItemTool;

/// The label facts an athlete reads off a product the catalogue lacks, per
/// serving. Protein, fat and potassium are read when sent but not advertised:
/// fuelling turns on carbohydrate, sodium, caffeine and fluid.
fn facts_property() -> PropertySchema {
    let mut p = BTreeMap::new();
    p.insert(
        "serving_label".to_owned(),
        string_property("'1 gel (40 g)'"),
    );
    for (key, text) in [
        ("serving_g", "g"),
        ("carbs_g", "g"),
        ("sugars_g", "g"),
        ("energy_kcal", "kcal"),
        ("sodium_mg", "mg"),
        ("caffeine_mg", "mg"),
        ("fluid_ml", "ml"),
        ("glucose_fructose_ratio", "1:0.8 is 0.8"),
    ] {
        p.insert(key.to_owned(), number_property(text));
    }
    PropertySchema {
        property_type: "object".to_owned(),
        description: Some("Per serving, as printed; never estimate.".to_owned()),
        properties: Some(p),
        ..Default::default()
    }
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for AddFuelItemTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "product_id".to_owned(),
            string_property("A search_fuel_products product_id."),
        );
        properties.insert("brand".to_owned(), string_property("New product only."));
        properties.insert("name".to_owned(), string_property("Product name."));
        properties.insert("variant".to_owned(), string_property("Flavour."));
        properties.insert(
            "form".to_owned(),
            string_property(&format!("One of: {}.", FORMS.join(", "))),
        );
        properties.insert("gtin".to_owned(), string_property("Barcode, if known."));
        properties.insert("facts".to_owned(), facts_property());
        properties.insert("note".to_owned(), string_property("How they use it."));
        let schema = object_schema(properties, None);
        answers_with::<AddFuelItemResult>(tool_definition(
            "add_fuel_item",
            "Add a gel, drink mix or bar to the athlete's fuel list: a product_id, or brand, name, form and label facts for one not in the catalogue, filed for review and unused until approved.",
            schema,
            annotations(Effect::AdditiveWrite, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let (tenant, user) = caller(&context)?;
            let note = optional_text(&args, "note")?;
            let repos = context.resources.repos();
            let product_id = match parse_id(&args, "product_id")? {
                Some(id) => Some(id),
                // A barcode the catalogue already holds is that product,
                // whatever else was typed.
                None => match typed_gtin(&args)? {
                    Some(gtin) => repos
                        .fuel_catalogue
                        .get_fuel_product_by_gtin(&gtin)
                        .await?
                        .map(|product| product.product.id),
                    None => None,
                },
            };
            if let Some(product_id) = product_id {
                let carried = repos
                    .athlete_fuel_items
                    .add_fuel_product_item(tenant, user, product_id, note.as_deref())
                    .await?;
                return answer_carried(&context, tenant, user, carried).await;
            }
            match proposal_from_args(&args, user)? {
                Ok(proposal) => {
                    let items = repos
                        .athlete_fuel_items
                        .list_fuel_items(tenant, user)
                        .await?;
                    // The same product filed again while its first filing is
                    // still under review is already on the list: a second
                    // proposal would only queue the label twice.
                    if let Some(held) = pending_filing_of(&items, &proposal) {
                        return answer_carried(
                            &context,
                            tenant,
                            user,
                            FuelItemCarried::AlreadyCarried(held),
                        )
                        .await;
                    }
                    // Filed and carried in one transaction: at the item cap
                    // the filing is refused with it, never left queued alone.
                    let item_id = repos
                        .athlete_fuel_items
                        .file_and_carry_fuel_proposal(tenant, user, &proposal, note.as_deref())
                        .await?;
                    answer_carried(&context, tenant, user, FuelItemCarried::Added(item_id)).await
                }
                Err(refusal) => Ok(refusal),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The proposal an athlete's typed product files, or the refusal of facts the
/// plausibility gate rejects.
fn proposal_from_args(args: &Value, user: Uuid) -> AppResult<Result<NewFuelProposal, ToolResult>> {
    let (Some(brand), Some(name), Some(form)) = (
        optional_text(args, "brand")?,
        optional_text(args, "name")?,
        optional_text(args, "form")?,
    ) else {
        return Err(AppError::invalid_input(
            "give product_id from search_fuel_products, or brand, name, form and facts for a \
             product the catalogue does not hold",
        ));
    };
    let form = FuelForm::parse(&form).ok_or_else(|| {
        AppError::invalid_input(format!("form '{form}' is not one of: {}", FORMS.join(", ")))
    })?;
    let typed: FuelLabelFacts = match args.get("facts") {
        Some(facts) if !facts.is_null() => serde_json::from_value(facts.clone())
            .map_err(|e| AppError::invalid_input(format!("facts: {e}")))?,
        _ => FuelLabelFacts::default(),
    };
    if let Some(label) = typed.serving_label.as_deref() {
        if label.chars().count() > MAX_TEXT_LEN {
            return Err(AppError::invalid_input(format!(
                "facts.serving_label is longer than {MAX_TEXT_LEN} characters"
            )));
        }
    }
    let facts = typed.to_facts();
    if let Err(violations) = check_plausibility(form, &facts) {
        let codes: Vec<&str> = violations.iter().map(PlausibilityViolation::code).collect();
        return Ok(Err(ToolResult::error(json!({
            "reason": "implausible_label",
            "violations": codes,
            "error": format!(
                "These facts cannot be what a {} label states ({}). Nothing was filed: ask \
                 the athlete to read the label again rather than adjusting a value.",
                form.as_str(),
                codes.join(", ")
            ),
        }))));
    }
    let provenance: FactProvenance = typed
        .stated_fields()
        .into_iter()
        .map(|field| {
            (
                field.to_owned(),
                FieldProvenance {
                    source: FactSource::Athlete,
                    source_url: None,
                    confidence: None,
                },
            )
        })
        .collect();
    Ok(Ok(NewFuelProposal {
        product_id: None,
        brand,
        name,
        variant: optional_text(args, "variant")?.unwrap_or_default(),
        form,
        gtin: typed_gtin(args)?,
        source_kind: ProposalSource::Athlete,
        source_url: None,
        images: Vec::new(),
        notes: None,
        ingredients_text: None,
        multiple_transportable_stated: None,
        source_hash: None,
        facts,
        provenance,
        gate_flags: Vec::new(),
        proposed_by_user_id: Some(user),
    }))
}

/// The barcode typed, as the GTIN-14 the catalogue stores. A barcode that is
/// not a valid UPC, EAN or GTIN is refused rather than dropped: a misread
/// digit would otherwise file the product with no barcode and say nothing.
fn typed_gtin(args: &Value) -> AppResult<Option<String>> {
    optional_text(args, "gtin")?
        .map(|raw| {
            normalize_gtin(&raw).ok_or_else(|| {
                AppError::invalid_input(format!(
                    "gtin '{raw}' is not a valid UPC, EAN or GTIN barcode (wrong length or check \
                     digit): ask the athlete to read it again, or leave it out"
                ))
            })
        })
        .transpose()
}

/// The athlete's live item that carries a still-pending filing of the same
/// product (brand, name, variant and form), if any.
fn pending_filing_of(items: &[AthleteFuelItem], proposal: &NewFuelProposal) -> Option<Uuid> {
    let key = fuel_natural_key(&proposal.brand, &proposal.name, &proposal.variant);
    items.iter().find_map(|item| match &item.entry {
        FuelItemEntry::Pending(pending)
            if pending.status == ProposalStatus::Pending
                && pending.form == proposal.form
                && fuel_natural_key(&pending.brand, &pending.name, &pending.variant) == key =>
        {
            Some(item.id)
        }
        _ => None,
    })
}

async fn answer_carried(
    context: &ToolExecutionContext,
    tenant: TenantId,
    user: Uuid,
    carried: FuelItemCarried,
) -> AppResult<ToolResult> {
    let item = listed_item(context, tenant, user, carried.id()).await?;
    let message = match item.status {
        FuelItemStatus::Approved => {
            "On the athlete's list with its approved label facts; a plan day may name its product_id."
        }
        FuelItemStatus::Pending => {
            "On the athlete's list, pending review: its facts are not checked yet and will not be \
             used for any fuelling arithmetic or plan until an operator approves them. Tell the \
             athlete so."
        }
        FuelItemStatus::Rejected => {
            "This product was rejected at review: its facts are not used. Tell the athlete."
        }
    };
    ok_typed(
        "add_fuel_item",
        AddFuelItemResult {
            item,
            already_carried: matches!(carried, FuelItemCarried::AlreadyCarried(_)),
            message: message.to_owned(),
        },
    )
}

// ============================================================================
// list_fuel_items
// ============================================================================

/// The athlete's fuel items.
#[derive(Debug)]
pub struct ListFuelItemsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListFuelItemsTool {
    fn definition(&self) -> Tool {
        let schema = object_schema(HashMap::new(), None);
        answers_with::<ListFuelItemsResult>(tool_definition(
            "list_fuel_items",
            "The gels, drink mixes and bars on the athlete's fuel list: approved ones with label facts, others pending review or rejected (facts not used).",
            schema,
            annotations(Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let (tenant, user) = caller(&context)?;
            let items: Vec<FuelItemView> = context
                .resources
                .repos()
                .athlete_fuel_items
                .list_fuel_items(tenant, user)
                .await?
                .iter()
                .map(FuelItemView::from_item)
                .collect();
            ok_typed(
                "list_fuel_items",
                ListFuelItemsResult {
                    count: items.len(),
                    items,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// remove_fuel_item
// ============================================================================

/// Stop carrying a fuel item.
#[derive(Debug)]
pub struct RemoveFuelItemTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for RemoveFuelItemTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "item_id".to_owned(),
            string_property("A list_fuel_items item_id."),
        );
        let schema = object_schema(properties, Some(vec!["item_id".to_owned()]));
        answers_with::<RemoveFuelItemResult>(tool_definition(
            "remove_fuel_item",
            "Take a product off the athlete's fuel list.",
            schema,
            annotations(Effect::IdempotentWrite, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let (tenant, user) = caller(&context)?;
            let item_id = parse_id(&args, "item_id")?
                .ok_or_else(|| AppError::invalid_input("item_id is required"))?;
            let removed = context
                .resources
                .repos()
                .athlete_fuel_items
                .remove_fuel_item(tenant, user, item_id)
                .await?;
            ok_typed(
                "remove_fuel_item",
                RemoveFuelItemResult {
                    item_id: item_id.to_string(),
                    removed,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// The four fuel-product tools, registered in the `nutrition` category.
#[must_use]
pub fn create_fuel_product_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(SearchFuelProductsTool),
        Box::new(AddFuelItemTool),
        Box::new(ListFuelItemsTool),
        Box::new(RemoveFuelItemTool),
    ]
}

// Catalogue and list text was typed by athletes or read off labels by OCR, so
// it re-enters the prompt as untrusted data. A removal keeps the row and the
// athlete can carry the product again, so it is reversible, like
// `commitment_cancel`.
crate::declare_security!(SearchFuelProductsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(AddFuelItemTool => UNTRUSTED_OUTPUT);
crate::declare_security!(ListFuelItemsTool => UNTRUSTED_OUTPUT);
crate::declare_security!(RemoveFuelItemTool => empty);
