// ABOUTME: The athlete's fuel products rendered into the system prompt — approved ones with label facts, pending and rejected ones without
// ABOUTME: Bounded, sanitized, and withheld exactly where the plan block is: during an interview and in a room (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fuel-products prompt block
//!
//! The gels, drink mixes and bars the athlete carries (`athlete_fuel_items`)
//! as a `## Fuel products the athlete carries` section, between their
//! commitments and their training plan, so session fuelling is built from
//! what they actually have and a plan day can name a product by id.
//!
//! - **Figures only from approved versions.** A catalogue product prints its
//!   current label facts per serving; a product the athlete filed is listed
//!   as pending review, and a rejected one as rejected, both without figures:
//!   a fact no operator approved is never something the agent quotes.
//! - **Bounded.** At most `FUEL_PRODUCTS_PROMPT_LIMIT` items, the rest
//!   pointed at `list_fuel_items`.
//! - **Untrusted text, flattened.** Brand, name, variant and note were typed
//!   by athletes or read off labels; each passes the plan block's
//!   `sanitize_prompt_field` before it enters the unfenced prompt.
//! - **Withheld where the plan is.** During an interview, whose directive the
//!   block would derail, and on a room turn, whose reply every member reads.
//!   In a coach's thread about one athlete it is that athlete's list.
//! - **No medical-flag gate.** Label facts describe a product and set no
//!   amount for the athlete, as the label-describing nutrition tools are not
//!   gated; what a plan prescribes from them is.
//!
//! Like the plan block's instructions, the section's few lines of guidance
//! are written here, beside the data they introduce.

use std::fmt::Write as _;

use pierre_core::models::{AthleteFuelItem, FuelFacts, FuelItemEntry, ProposalStatus, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_services::plan_fuel_products::fuel_product_label;
use pierre_services::training_plan_render::sanitize_prompt_field;
use tracing::warn;
use uuid::Uuid;

/// The most fuel items the block lists.
pub(crate) const FUEL_PRODUCTS_PROMPT_LIMIT: usize = 12;

/// Longest product label or note the block prints.
const MAX_FIELD_LEN: usize = 120;

/// Who reads the block, which decides whether the fuel tools reach the list
/// it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FuelBlockReader {
    /// The athlete's own turn: `list_fuel_items` reads this same list.
    Athlete,
    /// A coach's thread about the athlete. The fuel tools act on the caller's
    /// own list, the coach's, so the block never points the agent at them.
    CoachThread,
}

/// Append the athlete's fuel-products block to `base_prompt`.
///
/// Nothing is appended when `withheld` or the athlete carries nothing.
/// Best-effort like the other subject blocks: a failed read is logged and the
/// prompt goes on without it.
pub async fn inject_fuel_products(
    repos: &RepositoryRegistry,
    (tenant, user): (TenantId, Uuid),
    reader: FuelBlockReader,
    withheld: bool,
    base_prompt: String,
) -> String {
    if withheld {
        return base_prompt;
    }
    match repos.athlete_fuel_items.list_fuel_items(tenant, user).await {
        Ok(items) => match render_fuel_products_block(&items, reader) {
            Some(block) => format!("{base_prompt}{block}"),
            None => base_prompt,
        },
        Err(e) => {
            warn!(error = %e, "fuel items read failed; continuing without them");
            base_prompt
        }
    }
}

/// The section for `items`, or `None` when there are none.
#[must_use]
pub(crate) fn render_fuel_products_block(
    items: &[AthleteFuelItem],
    reader: FuelBlockReader,
) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(512);
    out.push_str("\n\n## Fuel products the athlete carries\n\n");
    out.push_str(
        "Build session fuelling from these, and name an approved one on a plan day by its \
         product_id in `fuel_product_ids`. Figures are approved label facts per serving. A \
         product pending review has no approved facts yet: never quote amounts for it. A \
         rejected one was refused at review: tell the athlete if they name it.\n\n",
    );
    for item in items.iter().take(FUEL_PRODUCTS_PROMPT_LIMIT) {
        let _ = writeln!(out, "- {}", item_line(item));
    }
    if items.len() > FUEL_PRODUCTS_PROMPT_LIMIT {
        let more = items.len() - FUEL_PRODUCTS_PROMPT_LIMIT;
        let _ = match reader {
            FuelBlockReader::Athlete => {
                writeln!(out, "({more} more — read them with `list_fuel_items`.)")
            }
            FuelBlockReader::CoachThread => writeln!(
                out,
                "({more} more on the athlete's list, not shown here; `list_fuel_items` reads \
                 the coach's own list, never the athlete's.)"
            ),
        };
    }
    Some(out)
}

fn item_line(item: &AthleteFuelItem) -> String {
    let mut line = match &item.entry {
        FuelItemEntry::Catalogue(catalogued) => {
            let facts = facts_clause(&catalogued.current.facts);
            let product = &catalogued.product;
            format!(
                "{} — {}, product_id {}{}",
                sanitize_prompt_field(
                    &fuel_product_label(&product.brand, &product.name, &product.variant),
                    MAX_FIELD_LEN
                ),
                product.form.as_str(),
                product.id,
                if facts.is_empty() {
                    String::new()
                } else {
                    format!(": {facts}")
                }
            )
        }
        FuelItemEntry::Pending(pending) => {
            let label = fuel_product_label(&pending.brand, &pending.name, &pending.variant);
            let status = if pending.status == ProposalStatus::Rejected {
                "rejected at review"
            } else {
                "pending review, no approved facts"
            };
            format!(
                "{} — {} [{status}]",
                sanitize_prompt_field(&label, MAX_FIELD_LEN),
                pending.form.as_str()
            )
        }
    };
    if let Some(note) = item.note.as_deref().filter(|n| !n.trim().is_empty()) {
        let _ = write!(
            line,
            " · athlete's note: {}",
            sanitize_prompt_field(note, MAX_FIELD_LEN)
        );
    }
    line
}

/// One serving's facts as a clause: what fuelling turns on, the rest left to
/// `list_fuel_items`.
fn facts_clause(facts: &FuelFacts) -> String {
    let mut parts = Vec::new();
    if let Some(serving) = facts.serving_label.as_deref() {
        parts.push(format!(
            "per {}",
            sanitize_prompt_field(serving, MAX_FIELD_LEN)
        ));
    } else if let Some(grams) = facts.serving_g {
        parts.push(format!("per {grams} g serving"));
    }
    for (value, unit) in [
        (facts.carbs_g, "g carbs"),
        (facts.sodium_mg, "mg sodium"),
        (facts.caffeine_mg, "mg caffeine"),
        (facts.fluid_ml, "ml fluid"),
    ] {
        if let Some(value) = value {
            parts.push(format!("{value} {unit}"));
        }
    }
    if let Some(ratio) = facts.glucose_fructose_ratio {
        parts.push(format!("glucose:fructose 1:{ratio}"));
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::{FuelForm, PendingFuelItem};

    use super::*;

    /// `count` pending chews, in memory: the bound and the overflow line
    /// turn on how many items there are, never on what a store returned.
    fn chews(count: usize) -> Vec<AthleteFuelItem> {
        (0..count)
            .map(|n| AthleteFuelItem {
                id: Uuid::new_v4(),
                note: None,
                created_at: Utc::now(),
                entry: FuelItemEntry::Pending(PendingFuelItem {
                    proposal_id: Uuid::new_v4(),
                    brand: "Filler".to_owned(),
                    name: format!("Chew {n}"),
                    variant: String::new(),
                    form: FuelForm::Chew,
                    facts: FuelFacts::default(),
                    status: ProposalStatus::Pending,
                }),
            })
            .collect()
    }

    #[test]
    fn nothing_carried_renders_no_section() {
        assert_eq!(
            render_fuel_products_block(&[], FuelBlockReader::Athlete),
            None
        );
    }

    /// Past the limit the athlete's turn points the rest at
    /// `list_fuel_items`, which reads that same list.
    #[test]
    fn past_the_limit_the_athlete_is_pointed_at_list_fuel_items() {
        let block = render_fuel_products_block(
            &chews(FUEL_PRODUCTS_PROMPT_LIMIT + 2),
            FuelBlockReader::Athlete,
        )
        .expect("the athlete carries items");
        assert_eq!(
            block.lines().filter(|line| line.starts_with("- ")).count(),
            FUEL_PRODUCTS_PROMPT_LIMIT
        );
        assert!(
            block.contains("(2 more — read them with `list_fuel_items`.)"),
            "{block}"
        );
    }

    /// In a coach's thread the fuel tools read the coach's own list, so the
    /// overflow line never points the agent at them for the athlete's.
    #[test]
    fn past_the_limit_a_coach_thread_is_never_pointed_at_the_fuel_tools() {
        let block = render_fuel_products_block(
            &chews(FUEL_PRODUCTS_PROMPT_LIMIT + 2),
            FuelBlockReader::CoachThread,
        )
        .expect("the athlete carries items");
        assert_eq!(
            block.lines().filter(|line| line.starts_with("- ")).count(),
            FUEL_PRODUCTS_PROMPT_LIMIT
        );
        assert!(
            block.contains("(2 more on the athlete's list, not shown here;"),
            "{block}"
        );
        assert!(
            !block.contains("read them with `list_fuel_items`"),
            "{block}"
        );
    }
}
