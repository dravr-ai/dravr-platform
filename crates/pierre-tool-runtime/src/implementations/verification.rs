// ABOUTME: verify_claim MCP tool — exposes the claim-verification (bullshit detector) pipeline as a tool call
// ABOUTME: Coach persona can call it mid-turn to sanity-check a claim before emitting it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Verification Tools
//!
//! Currently ships one tool — [`VerifyClaimTool`] — that runs a caller-
//! provided claim through the claim-verification detector pipeline and returns a
//! structured verdict. Persistence of the verdict to `claim_verdicts` is
//! done by the same pipeline the dispatch hook uses, so the admin "flagged
//! claims" surface stays consistent whether a claim was verified by the
//! coach mid-turn or by the post-LLM sweep.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::claim_extractor::ExtractedClaim;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use pierre_services::claim_verification;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

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

/// Run the claim-verification detector pipeline on a single claim.
pub struct VerifyClaimTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for VerifyClaimTool {
    fn definition(&self) -> Tool {
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
        let schema = object_schema(
            properties,
            Some(vec!["claim".to_owned(), "category".to_owned()]),
        );
        answers_with::<VerifyClaimResult>(tool_definition(
            "verify_claim",
            "Check a factual claim against the claim-verification pipeline (rhetoric → deterministic bounds → personalized physiology → evidence corpus) and return a structured verdict with evidence strength and optional citations. Use this before emitting any physiological, nutrition, training, or supplement claim you are not certain about.",
            schema,
            Some(verify_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::WRITES_DATA,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
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
            let corpus = claim_verification::resolve_corpus(context.resources.evidence_registry());
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
                .repos()
                .claim_verdicts
                .insert_claim_verdict(&params)
                .await?;

            let payload = VerifyClaimResult {
                verdict_id: verdict.id,
                status: matches_status(outcome.status).to_owned(),
                evidence_strength: outcome.evidence_strength.as_str().to_owned(),
                layer_fired: outcome.layer_fired.as_str().to_owned(),
                confidence: outcome.confidence,
                explanation: outcome.explanation,
                evidence_refs: outcome.evidence_refs,
            };
            Ok(ToolResult::ok(serde_json::to_value(payload).map_err(
                |e| AppError::internal(format!("verify_claim result did not serialize: {e}")),
            )?))
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The structured verdict `verify_claim` answers with.
///
/// Both halves of the contract come from this type: `answers_with` derives the
/// declared `outputSchema` from it, and `execute` serializes an instance of it
/// as the payload. They cannot drift, because adding a field here changes both
/// and removing one stops compiling.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VerifyClaimResult {
    /// Stable identifier of the persisted verdict.
    pub verdict_id: String,
    /// Whether the claim was supported, contradicted, or left unverified.
    pub status: String,
    /// How strong the supporting evidence is.
    pub evidence_strength: String,
    /// Which layer of the pipeline decided — rhetoric, bounds, physiology, corpus.
    pub layer_fired: String,
    /// Confidence in the verdict, 0.0 to 1.0.
    pub confidence: f32,
    /// Why the pipeline reached this verdict, in the coach's words.
    pub explanation: String,
    /// Citations backing the verdict; absent when the layer that fired cites none.
    pub evidence_refs: Option<String>,
}

const fn matches_status(s: ClaimStatus) -> &'static str {
    s.as_str()
}

/// Build the claim-verification tools for registration.
#[must_use]
pub fn create_verification_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(VerifyClaimTool)]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(VerifyClaimTool => UNTRUSTED_OUTPUT);
