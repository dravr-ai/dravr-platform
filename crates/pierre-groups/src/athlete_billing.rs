// ABOUTME: The count, the cap and the charge for one group owner's athletes under their tenant plan
// ABOUTME: One count feeds both the group allowance check and the per-athlete add-on billed above it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete billing
//!
//! A coach brings each athlete into a group of their own, so the groups an
//! owner holds are the athletes they coach. A tier's [`GroupAllowance`] says
//! what happens past the included count: Starter refuses, Professional bills
//! each further athlete on the coach's subscription, Enterprise neither.
//!
//! [`AthleteBilling`] owns the three steps so they cannot disagree: one
//! count of the owner's active groups, read by both
//! [`ensure_room`](AthleteBilling::ensure_room), the cap, and
//! [`sync`](AthleteBilling::sync), the charge.

use std::sync::Arc;

use pierre_core::billing::{BillingProvider, ExtraAthletesRequest};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId};
use pierre_database::repositories::{
    CoachingGroupRepository, SubscriptionsRepository, TenantRepository,
};
use uuid::Uuid;

use crate::strategies::tier::{tier_strategy_for, GroupAllowance};

/// What [`AthleteBilling::sync`] did with an owner's per-athlete charge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AthleteCharge {
    /// The billing provider now bills `quantity` athletes above the plan's
    /// included allowance on this subscription (`0` clears the charge).
    Billed {
        /// The subscription the charge sits on.
        provider_subscription_id: String,
        /// Athletes billed above the included allowance.
        quantity: u64,
    },
    /// The owner holds no live subscription with the active billing provider
    /// on this tenant, so nothing was sent; `quantity` is what the plan would
    /// bill.
    Unbilled {
        /// Athletes above the included allowance that no subscription carries.
        quantity: u64,
    },
}

/// The count, the cap and the charge for one owner's athletes.
///
/// Shared (`Arc`) between [`crate::GroupService`], which consults it on every
/// path that adds or removes one of an owner's active groups, and the billing
/// webhook, which re-applies the charge whenever a subscription changes.
pub struct AthleteBilling {
    /// The group repository the registry shares with every service; its
    /// owner count is the one the cap and the charge both read.
    groups: Arc<dyn CoachingGroupRepository>,
    /// Shared registry repository: the tenant plan names the allowance.
    tenants: Arc<dyn TenantRepository>,
    /// Shared registry repository: the owner's subscription the charge sits on.
    subscriptions: Arc<dyn SubscriptionsRepository>,
    /// The process-wide billing provider, shared with the billing routes.
    provider: Arc<dyn BillingProvider>,
}

impl AthleteBilling {
    /// Build over the group, tenant and subscription repositories and the
    /// active billing provider.
    #[must_use]
    pub fn new(
        groups: Arc<dyn CoachingGroupRepository>,
        tenants: Arc<dyn TenantRepository>,
        subscriptions: Arc<dyn SubscriptionsRepository>,
        provider: Arc<dyn BillingProvider>,
    ) -> Self {
        Self {
            groups,
            tenants,
            subscriptions,
            provider,
        }
    }

    /// The active groups `owner_id` holds on `tenant_id`: the count the
    /// allowance check reads and the per-athlete charge bills.
    ///
    /// # Errors
    ///
    /// Returns a database error if the count fails.
    async fn active_groups(&self, owner_id: Uuid, tenant_id: TenantId) -> AppResult<i64> {
        self.groups
            .count_groups_for_owner(owner_id, tenant_id)
            .await
    }

    /// Refuse a group creation `allowance` does not cover.
    ///
    /// Below the included count every tier admits the group. At or above it,
    /// [`GroupAllowance::Capped`] refuses, and [`GroupAllowance::BilledAbove`]
    /// admits it only when the owner has an entitled subscription on this
    /// tenant to bill the extra athlete to — without one, the included count
    /// is a cap.
    ///
    /// # Errors
    ///
    /// Returns an `invalid_input` error naming the allowance when the creation
    /// is refused, or a database error on failure.
    pub async fn ensure_room(
        &self,
        owner_id: Uuid,
        tenant_id: TenantId,
        allowance: GroupAllowance,
    ) -> AppResult<()> {
        let included = match allowance {
            GroupAllowance::Unlimited => return Ok(()),
            GroupAllowance::Capped(count) | GroupAllowance::BilledAbove(count) => count,
        };
        let held = self.active_groups(owner_id, tenant_id).await?;
        if held < i64::try_from(included).unwrap_or(i64::MAX) {
            return Ok(());
        }

        if matches!(allowance, GroupAllowance::BilledAbove(_)) {
            let billable = self
                .subscription(owner_id, tenant_id)
                .await?
                .is_some_and(|subscription| subscription.is_entitled());
            if billable {
                return Ok(());
            }
            return Err(AppError::invalid_input(format!(
                "Group limit reached ({included} included). Each athlete above {included} is \
                 billed on an active subscription; start one to add more."
            )));
        }

        Err(AppError::invalid_input(format!(
            "Group limit reached ({included}). Upgrade your plan for more groups."
        )))
    }

    /// Bill the owner's current athletes above their plan's allowance.
    ///
    /// Reads the tenant plan and the owner's count of active groups, and
    /// sends the absolute count above the included allowance to the billing
    /// provider on the owner's live subscription. A plan with no per-athlete
    /// charge sends zero, which clears an add-on a previous plan left behind.
    ///
    /// # Errors
    ///
    /// Returns a database error, or the billing provider's error when it
    /// refuses the charge.
    pub async fn sync(&self, owner_id: Uuid, tenant_id: TenantId) -> AppResult<AthleteCharge> {
        let plan = self.tenants.get_by_id(tenant_id).await?.plan;
        let held = self.active_groups(owner_id, tenant_id).await?;
        let quantity = tier_strategy_for(&plan)
            .group_allowance()
            .billed_athletes(held);

        let live = self
            .subscription(owner_id, tenant_id)
            .await?
            .filter(|subscription| still_billing(subscription.status));
        let Some(provider_subscription_id) =
            live.and_then(|subscription| subscription.provider_subscription_id)
        else {
            return Ok(AthleteCharge::Unbilled { quantity });
        };

        self.provider
            .set_extra_athletes(&ExtraAthletesRequest {
                provider_subscription_id: provider_subscription_id.clone(),
                quantity,
            })
            .await?;
        Ok(AthleteCharge::Billed {
            provider_subscription_id,
            quantity,
        })
    }

    /// The owner's subscription on this tenant with the active billing
    /// provider, when the provider has assigned it a subscription id.
    async fn subscription(
        &self,
        owner_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<Option<Subscription>> {
        let provider = self.provider.name();
        Ok(self
            .subscriptions
            .get_subscription_by_user(owner_id)
            .await?
            .filter(|subscription| {
                subscription.tenant_id == tenant_id
                    && subscription.provider == provider
                    && subscription.provider_subscription_id.is_some()
            }))
    }
}

/// Whether the provider still invoices a subscription in this status, so a
/// charge set on it lands on an invoice. A canceled or expired subscription
/// has no further invoice.
const fn still_billing(status: SubscriptionStatus) -> bool {
    !matches!(
        status,
        SubscriptionStatus::Canceled | SubscriptionStatus::IncompleteExpired
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ended_subscriptions_stop_billing() {
        assert!(still_billing(SubscriptionStatus::Active));
        assert!(still_billing(SubscriptionStatus::Trialing));
        assert!(still_billing(SubscriptionStatus::PastDue));
        assert!(still_billing(SubscriptionStatus::Unpaid));
        assert!(still_billing(SubscriptionStatus::Incomplete));
        assert!(!still_billing(SubscriptionStatus::Canceled));
        assert!(!still_billing(SubscriptionStatus::IncompleteExpired));
    }
}
