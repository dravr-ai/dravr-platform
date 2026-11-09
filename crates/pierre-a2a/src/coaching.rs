// ABOUTME: The seam an A2A text message reaches the coaching pipeline through, implemented by the composition root
// ABOUTME: A trait here because the pipeline sits above this crate: the protocol names what a turn needs, never how it runs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coaching turns, as the protocol sees them.
//!
//! `pierre-chat-pipeline` depends on far more than this crate may, so the
//! protocol describes what a text message needs and the composition root
//! answers it through the one turn ladder every chat surface climbs. Nothing
//! here builds a prompt or reads the athlete's data.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use uuid::Uuid;

/// A reserved place in the instance's coaching capacity. Held for as long as
/// the turn runs, wherever it runs, and given back when dropped.
pub struct CoachingSlot {
    _holder: Box<dyn Any + Send>,
}

impl CoachingSlot {
    /// Wrap whatever keeps the place taken while it lives.
    #[must_use]
    pub fn new(holder: impl Any + Send) -> Self {
        Self {
            _holder: Box::new(holder),
        }
    }
}

impl fmt::Debug for CoachingSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CoachingSlot")
    }
}

/// One coaching turn the protocol admitted.
pub struct CoachingTurn {
    /// The athlete the token acts for.
    pub user_id: Uuid,
    /// The tenant the token's grant was authorized in.
    pub tenant_id: TenantId,
    /// The conversation the task's `contextId` names.
    pub conversation_id: String,
    /// The text the message carried.
    pub content: String,
    /// The token's own grant.
    pub grant: Vec<OAuthScope>,
}

// The content is the athlete's words: only its size prints.
impl fmt::Debug for CoachingTurn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoachingTurn")
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("conversation_id", &self.conversation_id)
            .field("content_len", &self.content.len())
            .field("grant", &self.grant)
            .finish()
    }
}

/// How a coaching turn ended without a reply.
#[derive(Debug)]
pub enum CoachingEnd {
    /// The turn failed; the text is what the caller may read of why.
    Failed(String),
    /// The caller canceled the task while the turn ran.
    Canceled,
}

/// A future that resolves once the task the turn runs for was canceled.
pub type CanceledSignal = Pin<Box<dyn Future<Output = ()> + Send>>;

/// The coaching pipeline, as an A2A text message reaches it.
#[async_trait]
pub trait CoachingTurns: Send + Sync {
    /// Reserve a place for one turn, or `None` right away when none is free.
    /// Never waits: a public caller is told to come back, not queued.
    fn try_reserve(&self) -> Option<CoachingSlot>;

    /// Whether the text is the postback of a tapped suggestion, which the turn
    /// would resolve to a command rather than read as a question.
    fn is_postback(&self, text: &str) -> bool;

    /// Refuse an athlete past their usage caps before a task opens.
    ///
    /// # Errors
    ///
    /// The quota refusal, or the repository error reading the counters.
    async fn check_quota(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<()>;

    /// The conversation a `contextId` names for this athlete in this tenant,
    /// or one opened under an id the client chose that no conversation holds.
    /// A message naming none continues the athlete's newest thread on this
    /// surface, opening one only when they hold none, so a client that never
    /// threads is one conversation rather than one per message.
    ///
    /// # Errors
    ///
    /// An invalid-input refusal when the id cannot be theirs, saying nothing
    /// about whether it exists; the conversation cap's quota refusal.
    async fn open_context(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        context_id: Option<&str>,
    ) -> AppResult<String>;

    /// Run one turn to its reply text.
    ///
    /// # Errors
    ///
    /// How the turn ended without one.
    async fn run(
        &self,
        turn: CoachingTurn,
        slot: CoachingSlot,
        canceled: CanceledSignal,
    ) -> Result<String, CoachingEnd>;
}
