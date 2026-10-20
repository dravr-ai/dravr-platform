// ABOUTME: Professional plan: 15 athletes included, each one above billed on the coach's subscription
// ABOUTME: Drives REST group create/delete/deactivate and the billing webhook against a recording provider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A Professional coach brings each athlete into a group of their own. The
//! plan includes 15, and each athlete above 15
//! is billed as one unit of the per-athlete add-on on the coach's
//! subscription, and the count the charge bills is the count the cap reads.
//!
//! The billing provider here records every charge the platform sends it,
//! so the tests read the exact quantity a real provider would be told. It can
//! also refuse each charge, as a provider that is down or rejects the add-on
//! price does.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::http::StatusCode;
use chrono::{TimeZone, Utc};
use common::{
    create_test_server_resources_with_billing_provider, create_test_user_with_plan,
    generate_test_token,
};
use helpers::axum_test::AxumTestRequest;
use helpers::notify_capture::{capture_logs, capture_notify, named};
use pierre_core::billing::{
    BillingEvent, BillingProvider, CheckoutRequest, CheckoutResponse, EventEnvelope,
    ExtraAthletesRequest, Invoice, PortalRequest, PortalResponse, SubscriptionEventPayload,
    WebhookPayload,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::{Subscription, SubscriptionStatus, TenantId, UserTier};
use pierre_groups::service::ChannelGroupSpec;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_billing::billing_routes;
use pierre_routes_groups::GroupRoutes;
use serde_json::{json, Value};
use tracing::Level;
use uuid::Uuid;

const PROVIDER: &str = "recording";

/// Start of the billing period the webhooks report (2027-01-01T00:00:00Z).
const PERIOD_START: i64 = 1_798_761_600;

/// Length of one monthly billing period, in seconds.
const PERIOD_SECS: i64 = 30 * 24 * 60 * 60;

/// A billing provider that records every per-athlete charge it is asked to
/// set. Its webhook carries a plain JSON subscription in place of a signed
/// vendor payload: signature verification is each real provider's own
/// concern, covered by its own tests.
#[derive(Default)]
struct RecordingBilling {
    charges: Mutex<Vec<ExtraAthletesRequest>>,
    /// Refuse every charge once it is recorded.
    refuse_charges: bool,
}

impl RecordingBilling {
    fn charges(&self) -> Vec<(String, u64)> {
        self.charges
            .lock()
            .unwrap()
            .iter()
            .map(|c| (c.provider_subscription_id.clone(), c.quantity))
            .collect()
    }

    fn last_charge(&self) -> (String, u64) {
        self.charges()
            .last()
            .cloned()
            .expect("the platform set a per-athlete charge")
    }
}

#[async_trait]
impl BillingProvider for RecordingBilling {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    async fn start_checkout(&self, req: &CheckoutRequest) -> AppResult<CheckoutResponse> {
        Err(AppError::internal(format!(
            "checkout ({}) is not part of these tests",
            req.tier
        )))
    }

    async fn open_portal(&self, req: &PortalRequest) -> AppResult<PortalResponse> {
        Err(AppError::internal(format!(
            "the portal ({}) is not part of these tests",
            req.provider_customer_id
        )))
    }

    async fn parse_webhook(&self, payload: WebhookPayload<'_>) -> AppResult<EventEnvelope> {
        let body: Value = serde_json::from_slice(payload.body)
            .map_err(|e| AppError::invalid_input(e.to_string()))?;
        let field = |key: &str| body[key].as_str().unwrap().to_owned();
        let period_start = Utc
            .timestamp_opt(body["period_start"].as_i64().unwrap(), 0)
            .unwrap();
        Ok(EventEnvelope {
            event_id: field("id"),
            event_type: "subscription.upserted".to_owned(),
            event: BillingEvent::SubscriptionUpserted(Box::new(SubscriptionEventPayload {
                provider_customer_id: field("customer"),
                provider_subscription_id: Some(field("subscription")),
                tenant_id: field("tenant_id"),
                user_id: field("user_id"),
                plan_tier: field("plan_tier"),
                status: "active".to_owned(),
                current_period_start: Some(period_start),
                current_period_end: Some(period_start + chrono::Duration::seconds(PERIOD_SECS)),
                cancel_at_period_end: false,
                canceled_at: None,
                trial_end: None,
                metadata: None,
            })),
            occurred_at: period_start,
        })
    }

    async fn list_invoices(&self, provider_customer_id: &str) -> AppResult<Vec<Invoice>> {
        Err(AppError::internal(format!(
            "invoices ({provider_customer_id}) are not part of these tests"
        )))
    }

    async fn set_extra_athletes(&self, req: &ExtraAthletesRequest) -> AppResult<()> {
        self.charges.lock().unwrap().push(req.clone());
        if self.refuse_charges {
            return Err(AppError::external_service(
                PROVIDER,
                "the per-athlete price was refused",
            ));
        }
        Ok(())
    }
}

/// A coach on `plan`, signed in, with an athlete-facing agent to give groups.
struct Coach {
    res: Arc<ServerContext>,
    billing: Arc<RecordingBilling>,
    router: axum::Router,
    auth: String,
    user_id: Uuid,
    tenant_id: TenantId,
    agent_id: String,
}

async fn coach(email: &str, plan: &str) -> Coach {
    coach_billed_by(email, plan, RecordingBilling::default()).await
}

/// [`coach`], billed through `billing`.
async fn coach_billed_by(email: &str, plan: &str, billing: RecordingBilling) -> Coach {
    let billing = Arc::new(billing);
    let res = create_test_server_resources_with_billing_provider(
        Arc::clone(&billing) as Arc<dyn BillingProvider>
    )
    .await
    .unwrap();
    let (user_id, user, tenant_id) = create_test_user_with_plan(&res.agent.database, email, plan)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(&res, &user).await);
    let request: CreateAgentRequest = serde_json::from_value(
        json!({"title":"Test Coach","system_prompt":"Test.","category":"training","tags":["run"]}),
    )
    .unwrap();
    let agent_id = res
        .common
        .repos
        .agents
        .create(user_id, tenant_id, &request)
        .await
        .unwrap()
        .id
        .to_string();
    let router = GroupRoutes::routes(Arc::clone(&res))
        .merge(billing_routes::<ServerContext>().with_state(Arc::clone(&res)));
    Coach {
        res,
        billing,
        router,
        auth,
        user_id,
        tenant_id,
        agent_id,
    }
}

impl Coach {
    /// The coach's own live Professional subscription with the provider.
    async fn subscribe(&self, provider_subscription_id: &str) {
        let now = Utc::now();
        self.res
            .common
            .repos
            .subscriptions
            .upsert_subscription(&Subscription {
                id: Uuid::new_v4(),
                tenant_id: self.tenant_id,
                user_id: self.user_id,
                provider: PROVIDER.to_owned(),
                provider_customer_id: format!("cus_{}", self.user_id.simple()),
                provider_subscription_id: Some(provider_subscription_id.to_owned()),
                status: SubscriptionStatus::Active,
                plan_tier: UserTier::Professional,
                current_period_start: Some(now),
                current_period_end: Some(now + chrono::Duration::days(30)),
                cancel_at_period_end: false,
                canceled_at: None,
                trial_end: None,
                metadata: None,
                created_at: now,
                updated_at: now,
                last_event_at: None,
            })
            .await
            .unwrap();
    }

    /// `POST /api/groups` — one group, one athlete.
    async fn add_athlete(&self, n: usize) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post("/api/groups")
            .header("authorization", &self.auth)
            .json(&json!({ "name": format!("Athlete {n}"), "agent_id": self.agent_id }))
            .send(self.router.clone())
            .await;
        (resp.status_code(), resp.json())
    }

    /// Add athletes `from..=to`, each of which must be admitted.
    async fn add_athletes(&self, from: usize, to: usize) -> Vec<String> {
        let mut ids = Vec::new();
        for n in from..=to {
            let (status, body) = self.add_athlete(n).await;
            assert_eq!(status, StatusCode::CREATED, "athlete {n}: {body}");
            ids.push(body["id"].as_str().unwrap().to_owned());
        }
        ids
    }

    /// A subscription webhook naming this coach, as the provider delivers it,
    /// in the first billing period.
    async fn webhook(&self, event_id: &str, subscription: &str, plan_tier: &str) -> StatusCode {
        self.webhook_in_period(event_id, subscription, plan_tier, PERIOD_START)
            .await
    }

    /// [`Self::webhook`] for the billing period starting at `period_start`
    /// (Unix seconds).
    async fn webhook_in_period(
        &self,
        event_id: &str,
        subscription: &str,
        plan_tier: &str,
        period_start: i64,
    ) -> StatusCode {
        let resp = AxumTestRequest::post(&format!("/webhooks/{PROVIDER}"))
            .json(&json!({
                "id": event_id,
                "customer": format!("cus_{}", self.user_id.simple()),
                "subscription": subscription,
                "tenant_id": self.tenant_id.to_string(),
                "user_id": self.user_id.to_string(),
                "plan_tier": plan_tier,
                "period_start": period_start,
            }))
            .send(self.router.clone())
            .await;
        resp.status_code()
    }

    /// The active groups the coach holds, read from the repository the cap
    /// and the charge both count.
    async fn held_groups(&self) -> i64 {
        self.res
            .common
            .repos
            .groups
            .count_groups_for_owner(self.user_id, self.tenant_id)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_subscribed_professional_coach_is_billed_for_each_athlete_above_fifteen() {
    let coach = coach("pro-billed@test.com", "professional").await;
    coach.subscribe("sub_coach").await;

    coach.add_athletes(1, 15).await;
    let charges = coach.billing.charges();
    assert_eq!(
        charges.len(),
        15,
        "every group change re-applies the charge"
    );
    assert!(
        charges
            .iter()
            .all(|(sub, quantity)| sub == "sub_coach" && *quantity == 0),
        "the 15 included athletes bill nothing: {charges:?}"
    );

    // Athlete 16 is past the 15 included: admitted and billed.
    let (status, body) = coach.add_athlete(16).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "athlete 16 is billed, not refused: {body}"
    );
    assert_eq!(coach.billing.last_charge(), ("sub_coach".to_owned(), 1));

    let (status, body) = coach.add_athlete(17).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(coach.billing.last_charge(), ("sub_coach".to_owned(), 2));
}

#[tokio::test]
async fn removing_or_deactivating_an_athlete_lowers_the_charge() {
    let coach = coach("pro-lowered@test.com", "professional").await;
    coach.subscribe("sub_lowered").await;
    let groups = coach.add_athletes(1, 18).await;
    assert_eq!(coach.billing.last_charge(), ("sub_lowered".to_owned(), 3));

    let resp = AxumTestRequest::delete(&format!("/api/groups/{}", groups[0]))
        .header("authorization", &coach.auth)
        .send(coach.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);
    assert_eq!(
        coach.billing.last_charge(),
        ("sub_lowered".to_owned(), 2),
        "an archived group is no longer a billed athlete"
    );

    let resp = AxumTestRequest::put(&format!("/api/groups/{}", groups[1]))
        .header("authorization", &coach.auth)
        .json(&json!({ "is_active": false }))
        .send(coach.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(
        coach.billing.last_charge(),
        ("sub_lowered".to_owned(), 1),
        "a deactivated group is no longer a billed athlete"
    );

    // Back to the count the cap reads: 16 active groups, one above 15.
    assert_eq!(coach.held_groups().await, 16);
}

#[tokio::test]
async fn a_professional_coach_with_no_subscription_stays_capped_at_fifteen() {
    let coach = coach("pro-unsubscribed@test.com", "professional").await;

    coach.add_athletes(1, 15).await;
    let (status, body) = coach.add_athlete(16).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string()
            .contains("Group limit reached (15 included)"),
        "the refusal names the included athletes: {body}"
    );
    assert!(
        coach.billing.charges().is_empty(),
        "with no subscription there is nothing to send the provider"
    );
}

#[tokio::test]
async fn a_subscription_webhook_bills_the_athletes_a_coach_already_holds() {
    // A Starter coach whose bot sits in 17 chats: chat-bound groups never
    // stopped at a plan allowance, so the coach already holds 17 athletes
    // when they subscribe.
    let coach = coach("starter-subscribes@test.com", "starter").await;
    for n in 0..17 {
        coach
            .res
            .group_service()
            .create_channel_group(
                &ChannelGroupSpec {
                    name: "Athlete chat",
                    agent_id: &coach.agent_id,
                    channel_type: "telegram",
                    channel_chat_id: &format!("-100777{n:02}"),
                },
                coach.user_id,
                coach.tenant_id,
                2,
            )
            .await
            .unwrap();
    }
    assert!(
        coach.billing.charges().is_empty(),
        "no subscription yet, so nothing is billed"
    );

    assert_eq!(
        coach
            .webhook("evt_subscribed", "sub_late", "professional")
            .await,
        StatusCode::OK
    );
    assert_eq!(
        coach.billing.last_charge(),
        ("sub_late".to_owned(), 2),
        "the new Professional subscription bills the two athletes above 15"
    );

    // Moving to a plan with no per-athlete charge clears it.
    assert_eq!(
        coach
            .webhook("evt_enterprise", "sub_late", "enterprise")
            .await,
        StatusCode::OK
    );
    assert_eq!(coach.billing.last_charge(), ("sub_late".to_owned(), 0));
}

#[tokio::test]
async fn a_refused_charge_leaves_the_athlete_added_and_is_logged_as_an_error() {
    let coach = coach_billed_by(
        "pro-refused@test.com",
        "professional",
        RecordingBilling {
            refuse_charges: true,
            ..RecordingBilling::default()
        },
    )
    .await;
    coach.subscribe("sub_refused").await;
    coach.add_athletes(1, 15).await;

    let (lines, _guard) = capture_logs();
    let (status, body) = coach.add_athlete(16).await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "the group is written before the charge, so a refused charge does not undo it: {body}"
    );
    assert_eq!(coach.held_groups().await, 16);
    assert_eq!(
        coach.billing.last_charge(),
        ("sub_refused".to_owned(), 1),
        "the provider was asked for the athlete above 15"
    );
    let failures = named(&lines, "failed to apply the per-athlete charge");
    assert_eq!(failures.len(), 1, "the refusal reaches an operator once");
    assert_eq!(failures[0].level, Level::ERROR);
    assert_eq!(failures[0].field("owner_id"), coach.user_id.to_string());
    assert!(
        failures[0]
            .field("error")
            .contains("the per-athlete price was refused"),
        "the log carries the provider's reason: {:?}",
        failures[0].fields
    );
}

#[tokio::test]
async fn a_subscription_update_that_only_moves_the_add_on_announces_no_checkout() {
    let coach = coach("pro-announced@test.com", "professional").await;
    let (events, _guard) = capture_notify();

    assert_eq!(
        coach
            .webhook("evt_created", "sub_announced", "professional")
            .await,
        StatusCode::OK
    );
    assert_eq!(named(&events, "checkout.completed").len(), 1);

    // The provider re-sends the subscription when the add-on's quantity moves
    // with the coach's athletes: same status, plan and period.
    assert_eq!(
        coach
            .webhook("evt_add_on_moved", "sub_announced", "professional")
            .await,
        StatusCode::OK
    );
    assert_eq!(
        named(&events, "checkout.completed").len(),
        1,
        "an athlete added above 15 is not a checkout"
    );

    // A renewal moves the period, and is announced as before.
    assert_eq!(
        coach
            .webhook_in_period(
                "evt_renewed",
                "sub_announced",
                "professional",
                PERIOD_START + PERIOD_SECS,
            )
            .await,
        StatusCode::OK
    );
    assert_eq!(named(&events, "checkout.completed").len(), 2);
    assert_eq!(
        named(&events, "tier_changed").len(),
        1,
        "only the first subscription changed the tier"
    );
}
