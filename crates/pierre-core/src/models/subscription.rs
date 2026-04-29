// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Provider-agnostic subscription domain model — rows mirrored from any BillingProvider impl
// ABOUTME: Webhook handlers upsert keyed on (provider, provider_customer_id); status mirrors RFC-style lifecycle

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

use crate::models::{TenantId, UserTier};

/// Subscription lifecycle states.
///
/// Provider-agnostic — concrete `BillingProvider` impls map their own
/// status strings into one of these variants. Persisted as TEXT in
/// `SQLite` / TEXT (CHECK) in PG.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionStatus {
    /// In a free trial window — entitlement granted.
    Trialing,
    /// Paid and current — entitlement granted.
    Active,
    /// Most recent invoice failed; in dunning grace period.
    PastDue,
    /// Cancelled — entitlement revoked at `canceled_at`.
    Canceled,
    /// Initial payment is still pending confirmation.
    Incomplete,
    /// Initial payment never confirmed and the window expired.
    IncompleteExpired,
    /// Past dunning, never recovered.
    Unpaid,
}

impl SubscriptionStatus {
    /// Stable string form for DB storage and provider payload comparison.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trialing => "trialing",
            Self::Active => "active",
            Self::PastDue => "past_due",
            Self::Canceled => "canceled",
            Self::Incomplete => "incomplete",
            Self::IncompleteExpired => "incomplete_expired",
            Self::Unpaid => "unpaid",
        }
    }

    /// True when the subscription grants access to its plan tier.
    #[must_use]
    pub const fn is_entitled(self) -> bool {
        matches!(self, Self::Trialing | Self::Active)
    }
}

impl FromStr for SubscriptionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "trialing" => Ok(Self::Trialing),
            "active" => Ok(Self::Active),
            "past_due" => Ok(Self::PastDue),
            "canceled" => Ok(Self::Canceled),
            "incomplete" => Ok(Self::Incomplete),
            "incomplete_expired" => Ok(Self::IncompleteExpired),
            "unpaid" => Ok(Self::Unpaid),
            other => Err(format!("unknown subscription status: {other}")),
        }
    }
}

/// One row of `subscriptions`.
///
/// `(provider, provider_customer_id)` is the upsert key the webhook
/// dispatcher uses; `provider_subscription_id` becomes set on the
/// first subscription-created webhook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    /// Local primary key (UUID).
    pub id: Uuid,
    /// Tenant the subscription belongs to. Billing is per-tenant.
    pub tenant_id: TenantId,
    /// User who initiated checkout — receives portal/invoice access.
    pub user_id: Uuid,
    /// Provider slug — `stripe`, `revenuecat`, `dummy`, … — matching
    /// the `BillingProvider::name()` of the impl that owns this row.
    pub provider: String,
    /// Provider-side customer identifier; logically the upsert key for
    /// this row even when `provider_subscription_id` is still `None`.
    pub provider_customer_id: String,
    /// Provider-side subscription identifier; `None` between checkout
    /// and the first subscription-created webhook.
    pub provider_subscription_id: Option<String>,
    /// Current lifecycle state.
    pub status: SubscriptionStatus,
    /// Plan tier this subscription entitles when active/trialing.
    pub plan_tier: UserTier,
    /// Start of the current billing period (provider-supplied).
    pub current_period_start: Option<DateTime<Utc>>,
    /// End of the current billing period (provider-supplied).
    pub current_period_end: Option<DateTime<Utc>>,
    /// Whether the subscription will not auto-renew at period end.
    pub cancel_at_period_end: bool,
    /// Timestamp of the cancellation event, if any.
    pub canceled_at: Option<DateTime<Utc>>,
    /// End of the free trial, if the subscription is in trial.
    pub trial_end: Option<DateTime<Utc>>,
    /// JSON blob carrying any non-canonical provider metadata.
    pub metadata: Option<serde_json::Value>,
    /// Row creation timestamp (server local).
    pub created_at: DateTime<Utc>,
    /// Last-touched timestamp; bumped on every webhook upsert.
    pub updated_at: DateTime<Utc>,
}

impl Subscription {
    /// True when the subscription's status entitles its tier.
    #[must_use]
    pub const fn is_entitled(&self) -> bool {
        self.status.is_entitled()
    }
}
