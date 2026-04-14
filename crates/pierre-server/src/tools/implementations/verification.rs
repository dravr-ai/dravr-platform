// ABOUTME: Tier 5.5 verify_claim MCP tool — exposes the bullshit detector pipeline as a tool call
// ABOUTME: Coach persona can call it mid-turn to sanity-check a claim before emitting it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Verification Tools
//!
//! Currently ships one tool — [`VerifyClaimTool`] — that runs a caller-
//! provided claim through the Tier 5.5 detector pipeline and returns a
//! structured verdict. Persistence of the verdict to `claim_verdicts` is
//! done by the same pipeline the dispatch hook uses, so the admin "flagged
//! claims" surface stays consistent whether a claim was verified by the
//! coach mid-turn or by the post-LLM sweep.

use std::collections::HashMap;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::claim_extractor::ExtractedClaim;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength};
use serde_json::{json, Value};

use crate::errors::{AppError, AppResult};
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::services::claim_verification;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

/// Annotation set for read-but-persists-an-audit-row tools.
fn verify_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

fn parse_category(raw: &str) -> AppResult<ClaimCategory> {
    ClaimCategory::parse(raw)
        .ok_or_else(|| AppError::invalid_input(format!("unknown claim category '{raw}'")))
}

fn parse_strength(raw: &str) -> AppResult<EvidenceStrength> {
    EvidenceStrength::parse(raw)
        .ok_or_else(|| AppError::invalid_input(format!("unknown evidence strength '{raw}'")))
}

fn require_string_field(args: &Value, key: &str) -> AppResult<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_input(format!("missing required '{key}' field")))
}

fn optional_string_field(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Run the Tier 5.5 detector pipeline on a single claim.
pub struct VerifyClaimTool;

#[async_trait]
impl McpTool for VerifyClaimTool {
    fn name(&self) -> &'static str {
        "verify_claim"
    }

    fn description(&self) -> &'static str {
        "Check a factual claim against the Tier 5.5 bullshit detector pipeline (rhetoric → deterministic bounds → evidence corpus) and return a structured verdict with evidence strength and optional citations. Use this before emitting any physiological, nutrition, training, or supplement claim you are not certain about."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "claim".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The factual proposition to verify, as a single sentence.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "One of: physiological | training_prescription | nutrition | recovery | supplement | injury_rehab."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "minimum_strength".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Minimum evidence strength that counts as Supported. One of: strong | mixed | weak | none. Defaults to mixed."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "conversation_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional conversation id to attach the verdict row to.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Optional coach id for the verdict row.".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["claim".to_owned(), "category".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(verify_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = TenantId::from(context.require_tenant()?);
        let claim_text = require_string_field(&args, "claim")?;
        if claim_text.trim().is_empty() {
            return Err(AppError::invalid_input("claim must not be empty"));
        }
        if claim_text.len() > 2_000 {
            return Err(AppError::invalid_input(
                "claim exceeds 2000 character limit",
            ));
        }
        let category = parse_category(&require_string_field(&args, "category")?)?;
        let minimum_strength = optional_string_field(&args, "minimum_strength")
            .as_deref()
            .map(parse_strength)
            .transpose()?
            .unwrap_or(EvidenceStrength::Mixed);
        let conversation_id = optional_string_field(&args, "conversation_id");
        let coach_id = optional_string_field(&args, "coach_id");
        let user_id = context.user_id.to_string();

        let extracted = ExtractedClaim {
            text: claim_text.clone(),
            category,
        };
        let corpus = claim_verification::resolve_corpus(&context.resources);
        let outcome =
            claim_verification::verify_single_claim_with(&extracted, minimum_strength, &corpus);

        let params = InsertClaimVerdictParams {
            tenant_id,
            user_id: &user_id,
            coach_id: coach_id.as_deref(),
            conversation_id: conversation_id.as_deref(),
            message_id: None,
            claim_text: &claim_text,
            category,
            status: outcome.status,
            evidence_strength: outcome.evidence_strength,
            confidence: outcome.confidence,
            layer_fired: outcome.layer_fired,
            explanation: Some(&outcome.explanation),
            evidence_refs: outcome.evidence_refs.as_deref(),
        };
        let verdict = context
            .resources
            .repos
            .claim_verdicts
            .insert_claim_verdict(&params)
            .await?;

        Ok(ToolResult::ok(json!({
            "verdict_id": verdict.id,
            "status": matches_status(outcome.status),
            "evidence_strength": outcome.evidence_strength.as_str(),
            "layer_fired": outcome.layer_fired.as_str(),
            "confidence": outcome.confidence,
            "explanation": outcome.explanation,
            "evidence_refs": outcome.evidence_refs,
        })))
    }
}

const fn matches_status(s: ClaimStatus) -> &'static str {
    s.as_str()
}

/// Build the Tier 5.5 verification tools for registration.
#[must_use]
pub fn create_verification_tools() -> Vec<Box<dyn McpTool>> {
    vec![Box::new(VerifyClaimTool)]
}
