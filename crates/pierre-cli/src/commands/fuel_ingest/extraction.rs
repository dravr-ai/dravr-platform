// ABOUTME: Label transcription for `pierre-cli fuel-ingest` — the embacle call, the vision check on the tier that answered
// ABOUTME: Parses the fuel_panel_extraction JSON strictly and maps it to FuelFacts with per-field label_ocr provenance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The reading half of the ingestion job.
//!
//! [`read_label`] sends a candidate's photos through the platform's configured
//! chain (pierre-llm over embacle) with the contremaitre
//! `fuel_panel_extraction` prompt, routed as [`LlmStage::FuelPanelExtraction`].
//! A provider that cannot read images may still answer — embacle drops the
//! images and the model sees only text — so the tier that answered is checked:
//! an answer from a tier that does not advertise vision is discarded and
//! logged, never parsed, so a dropped image can never become invented values.
//!
//! The answer must be the prompt's JSON object. [`parse_reading`] accepts it
//! bare or inside one code fence and refuses anything else: no balanced-brace
//! hunt through prose, no field repair.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    check_plausibility, normalize_gtin, FactProvenance, FactSource, FieldProvenance, FuelFacts,
    FuelForm,
};
use pierre_core::redaction::redact_url;
use pierre_llm::served_tier::{observe_served_tier, ServedTier};
use pierre_llm::stage::LlmStage;
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, ImagePart, LlmCapabilities};
use serde::{Deserialize, Deserializer};
use tracing::{debug, info, warn};

use super::shopify::LabelImage;

/// Room for the whole JSON object, ingredients list included. A bilingual
/// Näak pouch's verbatim ingredients ran a 2,000-token answer out mid-object
/// (live dry run, 2026-10-29); the cap bounds a runaway, not a normal read.
const MAX_OUTPUT_TOKENS: u32 = 4_000;

/// A transcription, not a composition.
const TEMPERATURE: f32 = 0.0;

/// The `per_serving` block of the prompt's answer.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PerServing {
    pub energy_kcal: Option<f64>,
    pub carbs_g: Option<f64>,
    pub sugars_g: Option<f64>,
    pub protein_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub sodium_mg: Option<f64>,
    pub potassium_mg: Option<f64>,
    pub caffeine_mg: Option<f64>,
    pub fluid_ml: Option<f64>,
}

/// The `per_100g` block of the prompt's answer.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Per100g {
    pub carbs_g: Option<f64>,
    pub sugars_g: Option<f64>,
    pub protein_g: Option<f64>,
    pub fat_g: Option<f64>,
    pub sodium_mg: Option<f64>,
    pub salt_g: Option<f64>,
}

/// The form a reading names. A spelling this build does not know is `other`,
/// never a refused reading: the prompt is live config that may list a form
/// before the binary that parses it does, and the reviewer sees the form.
fn tolerant_form<'de, D>(deserializer: D) -> Result<FuelForm, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = String::deserialize(deserializer)?;
    Ok(FuelForm::parse(&raw).unwrap_or_else(|| {
        let shown: String = raw.chars().take(32).collect();
        warn!(form = %shown, "label form unknown to this build, read as other");
        FuelForm::Other
    }))
}

/// The object the `fuel_panel_extraction` prompt answers with. Unknown keys
/// are refused: an answer in another shape is not this prompt's answer. An
/// unknown `form` value is read as `other` ([`tolerant_form`]).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LabelReading {
    pub brand: Option<String>,
    pub name: Option<String>,
    pub variant: Option<String>,
    #[serde(deserialize_with = "tolerant_form")]
    pub form: FuelForm,
    pub gtin: Option<String>,
    pub serving_label: Option<String>,
    pub serving_g: Option<f64>,
    pub per_serving: PerServing,
    pub per_100g: Per100g,
    pub ingredients_text: Option<String>,
    pub glucose_fructose_ratio: Option<f64>,
    pub multiple_transportable_stated: bool,
    pub legible: bool,
    pub notes: Option<String>,
}

/// What a read label becomes: the facts, where each came from, the gate's
/// verdict and the identity the label prints.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MappedReading {
    pub facts: FuelFacts,
    pub provenance: FactProvenance,
    /// The [`pierre_core::models::PlausibilityViolation::code`]s the facts
    /// trip, each once, in the gate's order. The proposal is filed with them.
    pub gate_flags: Vec<String>,
    /// GTIN-14 when the printed digits are a valid GTIN.
    pub gtin: Option<String>,
}

/// Why a label produced no reading.
#[derive(Debug)]
pub(super) enum ReadFailure {
    /// The provider call failed on every tier.
    Provider(AppError),
    /// The tier that answered cannot read images; its answer was discarded.
    NoVision,
    /// The answer was not the prompt's JSON object.
    Unparseable(AppError),
}

/// A reading and the tier that produced it.
#[derive(Debug)]
pub(super) struct Reading {
    pub label: LabelReading,
    pub provider: &'static str,
    pub position: Option<usize>,
}

/// `text` without one surrounding code fence (with or without a `json` tag),
/// or `None` when it is not wrapped in exactly one.
fn unfenced(text: &str) -> Option<&str> {
    let inner = text.strip_prefix("```")?.strip_suffix("```")?;
    let inner = inner.strip_prefix("json").unwrap_or(inner);
    (!inner.contains("```")).then(|| inner.trim())
}

/// Parse the prompt's answer: the JSON object bare, or wrapped in one code
/// fence. Prose around it, a truncated object or a different shape is
/// refused.
///
/// # Errors
/// Returns an error naming where the JSON broke (the field path, line and
/// column) — never its text, which carries label content.
pub(super) fn parse_reading(content: &str) -> AppResult<LabelReading> {
    let trimmed = content.trim();
    let body = if trimmed.starts_with("```") {
        let body = unfenced(trimmed).ok_or_else(|| {
            AppError::invalid_input("label reading is not one fenced JSON object")
        })?;
        debug!("label reading came wrapped in a code fence");
        body
    } else {
        trimmed
    };
    let mut json = serde_json::Deserializer::from_str(body);
    let reading = serde_path_to_error::deserialize::<_, LabelReading>(&mut json).map_err(|e| {
        let inner = e.inner();
        AppError::invalid_input(format!(
            "label reading is not the prompt's JSON object: {:?} error at `{}`, line {} column {}",
            inner.classify(),
            e.path(),
            inner.line(),
            inner.column()
        ))
    })?;
    json.end().map_err(|e| {
        AppError::invalid_input(format!(
            "label reading has text after its JSON object, at line {} column {}",
            e.line(),
            e.column()
        ))
    })?;
    Ok(reading)
}

/// Set one fact and record it came from the label at `source_url`.
fn put(
    provenance: &mut FactProvenance,
    source_url: &str,
    name: &str,
    slot: &mut Option<f64>,
    value: Option<f64>,
) {
    if let Some(value) = value {
        *slot = Some(value);
        provenance.insert(
            name.to_owned(),
            FieldProvenance {
                source: FactSource::LabelOcr,
                source_url: Some(source_url.to_owned()),
                confidence: None,
            },
        );
    }
}

/// The facts `reading` states, each with `label_ocr` provenance pointing at
/// `source_url`, run through the plausibility gate. Values are carried as
/// read — the gate only names what it finds, so the reviewer sees why.
pub(super) fn map_reading(reading: &LabelReading, source_url: &str) -> MappedReading {
    let mut facts = FuelFacts::default();
    let mut provenance = FactProvenance::new();
    let p = &mut provenance;
    let s = &reading.per_serving;
    let h = &reading.per_100g;
    put(
        p,
        source_url,
        "serving_g",
        &mut facts.serving_g,
        reading.serving_g,
    );
    put(
        p,
        source_url,
        "energy_kcal",
        &mut facts.energy_kcal,
        s.energy_kcal,
    );
    put(p, source_url, "carbs_g", &mut facts.carbs_g, s.carbs_g);
    put(p, source_url, "sugars_g", &mut facts.sugars_g, s.sugars_g);
    put(
        p,
        source_url,
        "protein_g",
        &mut facts.protein_g,
        s.protein_g,
    );
    put(p, source_url, "fat_g", &mut facts.fat_g, s.fat_g);
    put(
        p,
        source_url,
        "sodium_mg",
        &mut facts.sodium_mg,
        s.sodium_mg,
    );
    put(
        p,
        source_url,
        "potassium_mg",
        &mut facts.potassium_mg,
        s.potassium_mg,
    );
    put(
        p,
        source_url,
        "caffeine_mg",
        &mut facts.caffeine_mg,
        s.caffeine_mg,
    );
    put(p, source_url, "fluid_ml", &mut facts.fluid_ml, s.fluid_ml);
    put(
        p,
        source_url,
        "carbs_g_per_100g",
        &mut facts.carbs_g_per_100g,
        h.carbs_g,
    );
    put(
        p,
        source_url,
        "sugars_g_per_100g",
        &mut facts.sugars_g_per_100g,
        h.sugars_g,
    );
    put(
        p,
        source_url,
        "protein_g_per_100g",
        &mut facts.protein_g_per_100g,
        h.protein_g,
    );
    put(
        p,
        source_url,
        "fat_g_per_100g",
        &mut facts.fat_g_per_100g,
        h.fat_g,
    );
    put(
        p,
        source_url,
        "sodium_mg_per_100g",
        &mut facts.sodium_mg_per_100g,
        h.sodium_mg,
    );
    put(
        p,
        source_url,
        "salt_g_per_100g",
        &mut facts.salt_g_per_100g,
        h.salt_g,
    );
    put(
        p,
        source_url,
        "glucose_fructose_ratio",
        &mut facts.glucose_fructose_ratio,
        reading.glucose_fructose_ratio,
    );
    if let Some(label) = reading
        .serving_label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        facts.serving_label = Some(label.to_owned());
        p.insert(
            "serving_label".to_owned(),
            FieldProvenance {
                source: FactSource::LabelOcr,
                source_url: Some(source_url.to_owned()),
                confidence: None,
            },
        );
    }

    let mut gate_flags: Vec<String> = Vec::new();
    if let Err(violations) = check_plausibility(reading.form, &facts) {
        for violation in violations {
            let code = violation.code().to_owned();
            if !gate_flags.contains(&code) {
                gate_flags.push(code);
            }
        }
    }

    let gtin = reading.gtin.as_deref().and_then(|printed| {
        let normalized = normalize_gtin(printed);
        if normalized.is_none() {
            warn!(
                source = %redact_url(source_url),
                digits = printed.len(),
                "label GTIN is not a valid GTIN (length or check digit); filed without one"
            );
        }
        normalized
    });

    MappedReading {
        facts,
        provenance,
        gate_flags,
        gtin,
    }
}

/// The request that asks for one candidate's label: the contremaitre prompt
/// as the system message, the photos on the user message.
///
/// # Errors
/// Returns an error when a photo's media type is one embacle refuses.
pub(super) fn label_request(
    provider: &ChatProvider,
    system_prompt: &str,
    images: &[LabelImage],
) -> AppResult<ChatRequest> {
    let parts = images
        .iter()
        .map(|image| {
            ImagePart::new(BASE64.encode(&image.bytes), image.mime.clone())
                .map_err(|e| AppError::invalid_input(format!("photo refused: {e}")))
        })
        .collect::<AppResult<Vec<_>>>()?;
    let request = ChatRequest::new(vec![
        ChatMessage::system(system_prompt.to_owned()),
        ChatMessage::user_with_images(
            format!(
                "Transcribe the label shown in these {} photo(s) of one package.",
                parts.len()
            ),
            parts,
        ),
    ])
    .with_temperature(TEMPERATURE)
    .with_max_tokens(MAX_OUTPUT_TOKENS);
    Ok(provider.routed(LlmStage::FuelPanelExtraction, request))
}

/// Who answered a label read and whether it could see the photos:
/// `(provider, chain position, reads images)`.
///
/// The served tier's own capabilities decide when the chain reported one. A
/// solo provider reports no served tier; its own capabilities stand. A chain
/// that answered without reporting its tier is unknown, so it reads as unable
/// to see: its answer is discarded rather than trusted.
fn answering_tier(
    served: Option<ServedTier>,
    solo: bool,
    provider_name: &'static str,
    provider_capabilities: LlmCapabilities,
) -> (&'static str, Option<usize>, bool) {
    match served {
        Some(tier) => (
            tier.provider,
            Some(tier.position),
            tier.capabilities.supports_vision(),
        ),
        None if solo => (provider_name, None, provider_capabilities.supports_vision()),
        None => (provider_name, None, false),
    }
}

/// Send `request` and parse the answer, keeping it only when the tier that
/// answered reads images ([`answering_tier`]).
pub(super) async fn read_label(
    provider: &ChatProvider,
    request: &ChatRequest,
    source_url: &str,
) -> Result<Reading, ReadFailure> {
    let (result, served) = observe_served_tier(provider.complete(request)).await;
    let response = result.map_err(ReadFailure::Provider)?;
    let (name, position, vision) = answering_tier(
        served,
        provider.fallback_tail().is_none(),
        provider.name(),
        provider.capabilities(),
    );
    if !vision {
        warn!(
            provider = name,
            position = ?position,
            model = %response.model,
            source = %redact_url(source_url),
            "label answer discarded: the tier that answered does not read images, so it never saw the label"
        );
        return Err(ReadFailure::NoVision);
    }
    info!(
        provider = name,
        position = ?position,
        model = %response.model,
        source = %redact_url(source_url),
        "label read"
    );
    parse_reading(&response.content)
        .map(|label| Reading {
            label,
            provider: name,
            position,
        })
        .map_err(ReadFailure::Unparseable)
}

#[cfg(test)]
mod tests {
    use super::*;

    const READING: &str = include_str!("fixtures/label_reading.json");
    const URL: &str = "https://naak.com/products/ultra-gel-200-salted-maple";

    fn reading() -> LabelReading {
        parse_reading(READING).unwrap_or_else(|e| panic!("fixture parses: {e}"))
    }

    #[test]
    fn the_prompts_object_parses_bare_and_in_one_fence() {
        let bare = reading();
        assert_eq!(bare.form, FuelForm::Gel);
        assert_eq!(bare.per_serving.carbs_g, Some(25.0));
        assert!(bare.legible);
        let fenced =
            parse_reading(&format!("```json\n{READING}\n```")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(fenced, bare);
    }

    #[test]
    fn prose_truncation_and_foreign_shapes_are_refused() {
        assert!(parse_reading(&format!("Here is the label:\n{READING}")).is_err());
        assert!(parse_reading(&READING[..READING.len() / 2]).is_err());
        assert!(parse_reading(&READING.replacen("\"legible\"", "\"readable\"", 1)).is_err());
        assert!(parse_reading(&READING.replacen("\"gel\"", "null", 1)).is_err());
        assert!(parse_reading(&format!("```json\n{READING}\n```\n```{{}}```")).is_err());
        assert!(parse_reading(&format!("{READING} {{}}")).is_err());
    }

    #[test]
    fn a_refused_reading_names_the_field_never_the_label_text() {
        let mistyped = READING.replacen("\"carbs_g\": 25", "\"carbs_g\": \"25 g net\"", 1);
        let message = match parse_reading(&mistyped) {
            Ok(reading) => panic!("a string carb amount was accepted: {reading:?}"),
            Err(e) => e.to_string(),
        };
        assert!(message.contains("`per_serving.carbs_g`"), "{message}");
        assert!(!message.contains("25 g net"), "{message}");
    }

    #[test]
    fn a_form_this_build_does_not_know_reads_as_other() {
        let syrup = parse_reading(&READING.replacen("\"gel\"", "\"syrup\"", 1))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(syrup.form, FuelForm::Other);
        for form in FuelForm::ALL {
            let named =
                parse_reading(&READING.replacen("\"gel\"", &format!("\"{}\"", form.as_str()), 1))
                    .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(named.form, form);
        }
    }

    #[test]
    fn a_reading_maps_to_facts_with_label_provenance() {
        let mapped = map_reading(&reading(), URL);
        assert_eq!(mapped.facts.serving_g, Some(57.0));
        assert_eq!(mapped.facts.carbs_g, Some(25.0));
        assert_eq!(mapped.facts.sodium_mg, Some(150.0));
        assert_eq!(mapped.facts.salt_g_per_100g, Some(0.66));
        assert_eq!(mapped.facts.glucose_fructose_ratio, Some(0.8));
        assert_eq!(mapped.facts.serving_label.as_deref(), Some("1 gel (57 g)"));
        // A value the label leaves out stays out, with no provenance.
        assert_eq!(mapped.facts.caffeine_mg, None);
        assert!(!mapped.provenance.contains_key("caffeine_mg"));
        let carbs = &mapped.provenance["carbs_g"];
        assert_eq!(carbs.source, FactSource::LabelOcr);
        assert_eq!(carbs.source_url.as_deref(), Some(URL));
        assert!(mapped.provenance.contains_key("serving_label"));
        assert!(mapped.provenance.contains_key("salt_g_per_100g"));
        assert_eq!(mapped.gtin.as_deref(), Some("04260512340018"));
        assert!(mapped.gate_flags.is_empty(), "{:?}", mapped.gate_flags);
    }

    #[test]
    fn an_implausible_label_is_flagged_never_clamped() {
        let mut implausible = reading();
        implausible.per_serving.sugars_g = Some(40.0);
        let mapped = map_reading(&implausible, URL);
        assert_eq!(mapped.facts.sugars_g, Some(40.0));
        // 40 g of 25 g carbs, and 40 g per serving disagrees with 21 g/100 g.
        assert!(
            mapped
                .gate_flags
                .contains(&"sugars_exceed_carbs".to_owned()),
            "{:?}",
            mapped.gate_flags
        );
    }

    fn tier(provider: &'static str, position: usize, capabilities: LlmCapabilities) -> ServedTier {
        ServedTier {
            provider,
            position,
            capabilities,
        }
    }

    #[test]
    fn only_a_tier_that_reads_images_is_trusted_with_a_label() {
        let text = LlmCapabilities::text_only();
        let vision = LlmCapabilities::full_featured();
        // A chain that fell back to a text-only tier: discarded, even though
        // its head reads images.
        assert_eq!(
            answering_tier(
                Some(tier("claude_code", 4, text)),
                false,
                "anthropic",
                vision
            ),
            ("claude_code", Some(4), false)
        );
        // The tier that answered reads images.
        assert_eq!(
            answering_tier(Some(tier("anthropic", 0, vision)), false, "anthropic", text),
            ("anthropic", Some(0), true)
        );
        // A solo provider reports no tier; its own capabilities stand.
        assert_eq!(
            answering_tier(None, true, "claude_code", text),
            ("claude_code", None, false)
        );
        assert_eq!(
            answering_tier(None, true, "anthropic", vision),
            ("anthropic", None, true)
        );
        // A chain that answered without naming its tier is not trusted, even
        // when its head reads images.
        assert_eq!(
            answering_tier(None, false, "anthropic", vision),
            ("anthropic", None, false)
        );
    }

    #[test]
    fn an_invalid_printed_gtin_is_dropped() {
        let mut bad = reading();
        bad.gtin = Some("4260512340017".to_owned());
        assert_eq!(map_reading(&bad, URL).gtin, None);
    }
}
