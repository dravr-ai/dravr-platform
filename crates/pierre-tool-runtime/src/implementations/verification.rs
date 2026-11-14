// ABOUTME: verify_claim MCP tool — exposes the claim-verification (bullshit detector) pipeline as a tool call
// ABOUTME: Judges a claim against the caller's own athlete data, so a client writing its own reply can check it first
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
//! agent mid-turn or by the post-LLM sweep.
//!
//! The chat pipeline checks the replies Dravr writes; an MCP client or an A2A
//! agent writes its own reply, which never reaches that stage. This tool is
//! how such a reply gets checked, so it judges the claim against the same
//! athlete context the chat pipeline does — the caller's physiology and own
//! activity record, built by
//! [`pierre_services::athlete_claim_context`] — and not against the
//! literature alone (carnet#852).
//!
//! Served outside Dravr's own surfaces, the tool judges and answers but files
//! nothing: the claim is a sentence the caller's assistant wrote, part of that
//! conversation, and Dravr keeps no copy of it (carnet#1565). On Dravr's own
//! surfaces the verdict row is written as before.

use crate::annotations::{annotations, Effect, Reach};
use pierre_providers::ai_scope;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_evals::claim_extractor::{ClaimSource, ExtractedClaim};
use pierre_evals::{PersonalizedConfig, PersonalizedContext};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use pierre_services::athlete_claim_context::{
    build_athlete_record, build_personalized_inputs, AthleteSources,
};
use pierre_services::claim_verification;

use crate::context::ToolExecutionContext;
use crate::conversions::{answers_with, object_schema, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::json_value::to_value_as_written;
use pierre_mcp_schema::PropertySchema;
use pierre_tools_core::ToolResult;

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
#[derive(Debug)]
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
                    "One of: athlete_data (a figure about this athlete's own training: a distance, duration, date or session) | physiological | training_prescription | nutrition | recovery | supplement | injury_rehab."
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
            "agent_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Optional agent id for the verdict row.".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(
            properties,
            Some(vec!["claim".to_owned(), "category".to_owned()]),
        );
        answers_with::<VerifyClaimResult>(tool_definition(
            "verify_claim",
            "Check a factual claim against the claim-verification pipeline (rhetoric, deterministic bounds, the athlete's own physiology and activity record, evidence corpus) and return a structured verdict with evidence strength and optional citations. Dravr checks the replies it writes in its own app and messaging; a reply written elsewhere from this server's data is checked only through this tool, and a call from outside Dravr's apps stores nothing.",
            schema,
            // Read-only to the caller. On Dravr's own surfaces the one row it
            // writes is its own verdict, in Dravr's claim-review log (the
            // admin "flagged claims" surface, and the conversation's verdict
            // list when the caller names a `conversation_id`); outside them it
            // writes nothing (carnet#1565). It never changes the athlete's
            // training, profile or settings.
            annotations("Verify factual claim", Effect::ReadOnly, Reach::Internal),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // It reads the athlete's activity record and physiology to judge the
        // claim, so a read grant is what it needs. The server instructions
        // and every data result's `_reply_verification` tell an external
        // client to call it, and the directory listing holds only a read
        // grant: declared as a write, it would be hidden from exactly those
        // clients (carnet#1510).
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
            let agent_id = optional_string_field(&args, "agent_id");
            let user_id = context.user_id.to_string();
            // A verdict filed under a conversation shows in that
            // conversation's verdict list, so the caller may name only a
            // conversation of their own.
            if let Some(conversation) = conversation_id.as_deref() {
                let owned = context
                    .resources
                    .repos()
                    .chat
                    .get_conversation(conversation, &user_id, tenant_id)
                    .await?;
                if owned.is_none() {
                    return Err(AppError::invalid_input(
                        "conversation_id is not one of your conversations",
                    ));
                }
            }

            let extracted = ExtractedClaim {
                text: claim_text.clone(),
                category,
                source: ClaimSource::Caller,
            };
            let corpus = context.resources.evidence_registry().resolved_corpus();

            // The caller's athlete context, from the builder the chat pipeline
            // checks Dravr's own replies with. Its reads run under this call's
            // provider terms and transport, so a verdict rests only on data
            // this caller may be served. They judge the claim and serve
            // nothing: a scope of their own keeps them from marking this call
            // as one that served the athlete's data (carnet#828).
            let sources = AthleteSources {
                repos: context.resources.repos().as_ref(),
                policies: context.resources.provider_registry().as_ref(),
                tenant_id,
                user_id: context.user_id,
            };
            let cageux = context.resources.cageux_config();
            let ((personalized_inputs, athlete_record), _) = ai_scope::serving_items(async {
                let personalized_inputs = build_personalized_inputs(
                    &sources,
                    &cageux.algorithms,
                    &context.resources.config().training_zones,
                    &PersonalizedConfig::default(),
                )
                .await;
                // No reply language to go by: the record reads the athlete's
                // own account locale.
                let athlete_record = build_athlete_record(&sources, None).await;
                (personalized_inputs, athlete_record)
            })
            .await;
            let personalized =
                personalized_inputs
                    .as_ref()
                    .map(|(metrics, tolerance)| PersonalizedContext {
                        metrics,
                        tolerance: tolerance.as_ref(),
                    });
            let outcome = claim_verification::verify_single_claim_with(
                &extracted,
                minimum_strength,
                &corpus,
                personalized.as_ref(),
                athlete_record.as_ref(),
            );

            // Outside Dravr's own surfaces the claim is the caller's
            // assistant's sentence, and no copy of it is kept (carnet#1565).
            let verdict_id = if ai_scope::serving_external() {
                None
            } else {
                let params = InsertClaimVerdictParams {
                    tenant_id,
                    user_id: &user_id,
                    agent_id: agent_id.as_deref(),
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
                    transport_policy: ai_scope::derived_policy(),
                };
                let verdict = context
                    .resources
                    .repos()
                    .claim_verdicts
                    .insert_claim_verdict(&params)
                    .await?;
                Some(verdict.id)
            };

            let payload = VerifyClaimResult {
                verdict_id,
                status: matches_status(outcome.status).to_owned(),
                evidence_strength: outcome.evidence_strength.as_str().to_owned(),
                layer_fired: outcome.layer_fired.as_str().to_owned(),
                confidence: outcome.confidence,
                explanation: outcome.explanation,
                evidence_refs: outcome.evidence_refs,
            };
            Ok(ToolResult::ok(to_value_as_written(&payload).map_err(
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
    /// Stable identifier of the persisted verdict. Absent when the call came
    /// from outside Dravr's own apps, where nothing is stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict_id: Option<String>,
    /// Whether the claim was supported, contradicted, or left unverified.
    pub status: String,
    /// How strong the supporting evidence is.
    pub evidence_strength: String,
    /// Which layer of the pipeline decided — `rhetoric`, `deterministic`,
    /// `personalized`, `athlete_data` or `evidence`.
    pub layer_fired: String,
    /// Confidence in the verdict, 0.0 to 1.0.
    pub confidence: f32,
    /// Why the pipeline reached this verdict, in the agent's words.
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
crate::declare_security!(VerifyClaimTool => empty);
