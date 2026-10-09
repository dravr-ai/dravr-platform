// ABOUTME: Seeds one conversation whose reply carries a supported claim verdict backed by a DOI, for the verdict-sheet flows
// ABOUTME: The athlete's question, the agent's reply quoting the claim, and the verdict row the reply's chip opens
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Verdict conversation seeder.
//!
//! A reply's verdict chip opens a sheet with the claim, the studies behind it
//! as links, the conversation the reply came from and an actions menu. A
//! verdict is written by the claim verifier after a live turn, so a stack with
//! no model behind it (the mobile E2E lanes) has none to open. This seeder
//! writes the rows such a turn leaves, so the mobile Maestro flow
//! `chat/11-verdict-sheet.yaml` has a chip to press:
//!
//! - a conversation titled [`CONVERSATION_TITLE`], owned by the athlete
//!   (`verdicttest@pierre.dev` by default, the account chat/11 signs in as);
//! - the athlete's question and the agent's reply, which states [`CLAIM`];
//! - one `supported` verdict on that reply, from the evidence layer, whose
//!   `evidence_refs` is [`EVIDENCE_REF`], a DOI the sheet links to doi.org.
//!
//! It is idempotent: an athlete who already has a conversation of that title is
//! left as they are.
//!
//! ```bash
//! pierre-cli seed verdict-conversation
//! ```

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::AddMessageParams;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::InsertClaimVerdictParams;
use pierre_database::RepositoryRegistry;
use pierre_memory::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_middleware::mask_email;
use tracing::info;

use crate::accounts::{account, Account};

/// The seeded conversation's title, which its list row and its header show.
pub const CONVERSATION_TITLE: &str = "Strength work for cycling";

/// The athlete's question.
pub const QUESTION: &str = "Should I add strength training to my cycling?";

/// The claim the verdict judged, as the reply states it.
pub const CLAIM: &str =
    "Heavy strength training improves cycling economy in trained endurance athletes";

/// The study behind the verdict: a DOI, which the sheet opens on doi.org.
pub const EVIDENCE_REF: &str = "doi:10.1111/sms.12104";

/// How many conversations are read when looking for an earlier seed.
const EXISTING_SCAN: i64 = 200;

/// CLI arguments for the verdict conversation seeder.
#[derive(clap::Args)]
pub struct SeedArgs {
    /// The athlete who owns the conversation
    #[arg(long, default_value = "verdicttest@pierre.dev")]
    pub email: String,

    /// The model the conversation is created with. `pierre-cli` fills it,
    /// when not given, with the model every new conversation gets from the
    /// configured LLM provider
    #[arg(long)]
    pub model: Option<String>,
}

/// The agent's reply: the claim in a sentence of its own, with the advice
/// around it, so the sheet's source preview has a passage to highlight it in.
fn reply() -> String {
    format!(
        "Yes, twice a week through the base phase. {CLAIM}, so the same power costs you less. \
         Keep the sets heavy and short, three to five reps, and leave a day between a gym \
         session and your hardest ride."
    )
}

/// Seed the conversation, its two messages and the verdict on the reply.
///
/// # Errors
///
/// Returns a configuration error when no model was resolved or the athlete is
/// missing, or an error if any repository write fails.
pub async fn run(args: SeedArgs, repos: &RepositoryRegistry) -> AppResult<()> {
    let model = args.model.ok_or_else(|| {
        AppError::config(
            "No model for the seeded conversation: pass --model, or configure the LLM \
             provider the server creates conversations with",
        )
    })?;
    let athlete = account(repos, &args.email).await?;
    if already_seeded(repos, &athlete).await? {
        info!(
            "{} already has {CONVERSATION_TITLE:?}; nothing to seed",
            mask_email(&args.email)
        );
        return Ok(());
    }

    let user_id = athlete.id.to_string();
    let conversation = repos
        .chat
        .create_conversation(
            &user_id,
            athlete.tenant,
            CONVERSATION_TITLE,
            &model,
            None,
            None,
        )
        .await?;
    let reply_text = reply();
    for (role, content) in [("user", QUESTION), ("assistant", reply_text.as_str())] {
        let message = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: athlete.tenant,
                conversation_id: &conversation.id,
                user_id: &user_id,
                role,
                content,
                token_count: None,
                finish_reason: (role == "assistant").then_some("stop"),
                prompt_tokens: None,
                model: (role == "assistant").then_some(model.as_str()),
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await?;
        if role == "assistant" {
            repos
                .claim_verdicts
                .insert_claim_verdict(&InsertClaimVerdictParams {
                    tenant_id: athlete.tenant,
                    user_id: &user_id,
                    agent_id: None,
                    conversation_id: Some(&conversation.id),
                    message_id: Some(&message.id),
                    claim_text: CLAIM,
                    category: ClaimCategory::TrainingPrescription,
                    status: ClaimStatus::Supported,
                    evidence_strength: EvidenceStrength::Strong,
                    confidence: 0.9,
                    layer_fired: VerdictLayer::Evidence,
                    explanation: Some(
                        "Reviews of heavy strength training in trained cyclists and runners \
                         report better economy with no loss of endurance capacity.",
                    ),
                    evidence_refs: Some(EVIDENCE_REF),
                    transport_policy: TransportPolicy::AnyTransport,
                })
                .await?;
        }
    }
    info!(
        "Seeded {CONVERSATION_TITLE:?} with a supported verdict for {}",
        mask_email(&args.email)
    );
    Ok(())
}

/// Whether the athlete already has a conversation of the seeded title.
async fn already_seeded(repos: &RepositoryRegistry, athlete: &Account) -> AppResult<bool> {
    let page = repos
        .chat
        .list_conversations(
            &athlete.id.to_string(),
            athlete.tenant,
            EXISTING_SCAN,
            0,
            TransportPolicy::FirstPartyOnly,
        )
        .await?;
    Ok(page
        .items
        .iter()
        .any(|conversation| conversation.title == CONVERSATION_TITLE))
}
