// ABOUTME: The vision's provenance snapshot — recommend_plan_flavour's verdict and inputs pass through save_training_plan
// ABOUTME: Pins the round-trip, the stored snapshot, the two notify events, and the refusal of an override wearing the rule's name
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The override rate against the rule is the vision's falsification test.
//! It can only be measured if what the rule proposed is stored beside what
//! was chosen, so the tool emits the kernel's own types, the save takes them
//! back verbatim, and every save with a flavour reports its provenance.

use anyhow::Result;
use pierre_core::models::periodization::{FlavourInputs, FlavourVerdict};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Debug as FmtDebug;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::subscriber::DefaultGuard;
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use uuid::Uuid;

mod common;

/// One tracing event with its recorded fields, captured off the notify
/// target so the test can assert what the save reported.
#[derive(Clone, Debug)]
struct CapturedEvent {
    fields: HashMap<String, String>,
}

#[derive(Clone, Default)]
struct CaptureLayer {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

#[derive(Default)]
struct FieldVisitor {
    fields: HashMap<String, String>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn FmtDebug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }
}

impl<S> Layer<S> for CaptureLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "notify" {
            return;
        }
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        if let Ok(mut events) = self.events.lock() {
            events.push(CapturedEvent {
                fields: visitor.fields,
            });
        }
    }
}

fn setup_capture() -> (Arc<Mutex<Vec<CapturedEvent>>>, DefaultGuard) {
    let capture = CaptureLayer::default();
    let events = Arc::clone(&capture.events);
    let guard = tracing_subscriber::registry().with(capture).set_default();
    (events, guard)
}

fn events_named<'a>(events: &'a [CapturedEvent], name: &str) -> Vec<&'a CapturedEvent> {
    events
        .iter()
        .filter(|e| e.fields.get("event").is_some_and(|v| v == name))
        .collect()
}

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

async fn create_test_user(executor: &UniversalToolExecutor) -> Result<(Uuid, String)> {
    let email = format!("vision_snapshot_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let tenants = executor.resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?;
    Ok((user_id, tenant.id.to_string()))
}

fn request(tool: &str, params: Value, user_id: Uuid, tenant_id: &str) -> UniversalRequest {
    UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: params,
        user_id: user_id.to_string(),
        protocol: "test".to_owned(),
        tenant_id: Some(tenant_id.to_owned()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    }
}

/// A plan payload whose flavour block is `flavour`. The 5 km runner at
/// eight hours from the tool's acceptance profiles, so the verdict is the
/// one the catalogue produces for a real athlete.
fn plan_payload(flavour: &Value) -> Value {
    json!({
        "coach_id": "endurance-coach",
        "outline": {
            "goal_race": { "name": "Parkrun PB", "date": "2026-11-14", "discipline": "run_5k", "priority": "A" },
            "strategy": "polarised build, two hard days, one long easy",
            "flavour": flavour,
            "phases": [
                {"kind": "build", "start": "2026-09-14", "weeks": 6, "intent": "VO2 and threshold, one each", "target_hours": 8.0},
                {"kind": "taper", "start": "2026-11-02", "weeks": 2, "intent": "sharpen"}
            ]
        },
        "weeks": [
            {
                "week_start": "2026-09-14",
                "focus": "first build week",
                "days": [
                    {"date": "2026-09-14", "sport": "rest", "workout": "off"},
                    {"date": "2026-09-15", "sport": "run", "workout": "5x3min VO2", "duration_min": 55, "intensity": "Z5"}
                ]
            }
        ]
    })
}

/// Run the rule for the 5 km runner and hand back its verdict and inputs
/// exactly as the tool returned them.
async fn recommendation(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: &str,
) -> Result<(Value, Value)> {
    let response = executor
        .execute_tool(request(
            "recommend_plan_flavour",
            json!({
                "hours_per_week": 8.0,
                "sessions_per_week": 5,
                "training_age": "trained",
                "event_class": "run_5k",
                "weeks_to_goal": 10,
                "measurements": ["hr", "pace"],
                "interval_experience": "two_seasons",
                "sport_mix": "running"
            }),
            user_id,
            tenant_id,
        ))
        .await?;
    assert!(response.success, "{:?}", response.error);
    let payload = response.result.expect("a payload");
    Ok((payload["verdict"].clone(), payload["inputs"].clone()))
}

#[tokio::test]
async fn the_tools_verdict_and_inputs_are_the_kernels_own_types() -> Result<()> {
    // The snapshot only works if what the tool returns is what the save can
    // read back: the shapes are the kernel's serde forms, not a hand-rolled
    // mirror that drifts.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let (verdict, inputs) = recommendation(&executor, user_id, &tenant_id).await?;

    let parsed: FlavourVerdict = serde_json::from_value(verdict.clone())?;
    assert_eq!(
        parsed.top().map(|s| s.id.as_str()),
        verdict["ranked"][0]["id"].as_str(),
        "the typed verdict ranks what the JSON ranks"
    );
    assert!(
        !parsed.excluded.is_empty(),
        "the 5 km runner cannot run every flavour"
    );

    let parsed: FlavourInputs = serde_json::from_value(inputs.clone())?;
    assert_eq!(parsed.sessions_per_week, 5);
    assert_eq!(parsed.weeks_to_goal, Some(10));
    assert!(
        inputs["sources"].is_array() && inputs["hours_tier"].is_string(),
        "the echo keeps its provenance and tier beside the typed fields: {inputs}"
    );
    Ok(())
}

#[tokio::test]
async fn a_rule_selection_stores_the_verdict_it_came_from_and_reports_it() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let (verdict, inputs) = recommendation(&executor, user_id, &tenant_id).await?;
    let top = verdict["ranked"][0]["id"]
        .as_str()
        .expect("a top flavour")
        .to_owned();

    let (events, guard) = setup_capture();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            plan_payload(&json!({
                "id": top,
                "selected_by": "rule",
                "verdict": verdict,
                "inputs": inputs
            })),
            user_id,
            &tenant_id,
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);
    assert!(saved.success, "save failed: {:?}", saved.error);

    let plan = executor
        .resources
        .repos()
        .training_plans
        .get_active_plan(&tenant_id, &user_id.to_string(), Some("endurance-coach"))
        .await?
        .expect("the plan is stored");
    let flavour = plan.flavour.expect("the flavour is stored");
    assert_eq!(flavour.id, top);
    let snapshot = flavour
        .verdict_snapshot
        .expect("the verdict is stored beside the choice");
    assert_eq!(snapshot.top().map(|s| s.id.clone()), Some(top.clone()));
    let inputs_snapshot = flavour.inputs_snapshot.expect("the inputs are stored too");
    assert_eq!(inputs_snapshot.weeks_to_goal, Some(10));

    let saved_events = events_named(&captured, "training_plan.vision_saved");
    assert_eq!(
        saved_events.len(),
        1,
        "one vision_saved per save: {captured:?}"
    );
    assert_eq!(
        saved_events[0].fields.get("flavour").map(String::as_str),
        Some(top.as_str())
    );
    assert_eq!(
        saved_events[0]
            .fields
            .get("selected_by")
            .map(String::as_str),
        Some("rule")
    );
    assert!(
        matches!(
            saved_events[0].fields.get("confidence").map(String::as_str),
            Some("low" | "moderate" | "high")
        ),
        "the confidence comes from the stored verdict: {:?}",
        saved_events[0].fields
    );
    assert!(
        events_named(&captured, "training_plan.flavour_overridden").is_empty(),
        "taking the rule's proposal is not an override"
    );
    Ok(())
}

#[tokio::test]
async fn an_override_reports_the_rules_pick_beside_the_humans() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let (verdict, inputs) = recommendation(&executor, user_id, &tenant_id).await?;
    let top = verdict["ranked"][0]["id"]
        .as_str()
        .expect("a top flavour")
        .to_owned();
    let other = verdict["ranked"]
        .as_array()
        .expect("ranked")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .find(|id| *id != top)
        .expect("a second eligible flavour to choose instead");

    let reason = "she thrives on the structure she already knows";
    let (events, guard) = setup_capture();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            plan_payload(&json!({
                "id": other,
                "selected_by": "coach",
                "override_reason": reason,
                "verdict": verdict,
                "inputs": inputs
            })),
            user_id,
            &tenant_id,
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);
    assert!(saved.success, "save failed: {:?}", saved.error);

    let overridden = events_named(&captured, "training_plan.flavour_overridden");
    assert_eq!(overridden.len(), 1, "{captured:?}");
    let fields = &overridden[0].fields;
    assert_eq!(
        fields.get("flavour").map(String::as_str),
        Some(other.as_str())
    );
    assert_eq!(
        fields.get("rule_top").map(String::as_str),
        Some(top.as_str())
    );
    assert_eq!(fields.get("selected_by").map(String::as_str), Some("coach"));
    assert_eq!(
        fields.get("reason_len").map(String::as_str),
        Some(reason.len().to_string().as_str()),
        "the reason's length travels, never its text: {fields:?}"
    );
    assert!(
        !fields.values().any(|v| v.contains("thrives")),
        "the reason's text never reaches the event: {fields:?}"
    );
    assert_eq!(
        events_named(&captured, "training_plan.vision_saved").len(),
        1
    );
    Ok(())
}

#[tokio::test]
async fn an_override_wearing_the_rules_name_is_refused() -> Result<()> {
    // selected_by rule with a verdict that ranked something else first is
    // an override the override rate would never count. The save names both.
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let (verdict, inputs) = recommendation(&executor, user_id, &tenant_id).await?;
    let top = verdict["ranked"][0]["id"]
        .as_str()
        .expect("a top flavour")
        .to_owned();
    let other = verdict["ranked"]
        .as_array()
        .expect("ranked")
        .iter()
        .map(|s| s["id"].as_str().expect("id").to_owned())
        .find(|id| *id != top)
        .expect("a second eligible flavour");

    let response = executor
        .execute_tool(request(
            "save_training_plan",
            plan_payload(&json!({
                "id": other,
                "selected_by": "rule",
                "verdict": verdict,
                "inputs": inputs
            })),
            user_id,
            &tenant_id,
        ))
        .await;
    let text = match response {
        Err(e) => e.to_string(),
        Ok(r) => {
            assert!(!r.success, "{:?}", r.result);
            format!("{:?} {:?}", r.error, r.result)
        }
    };
    assert!(
        text.contains("is not what the rule ranked first") && text.contains(&top),
        "the refusal names the rule's pick: {text}"
    );
    assert!(
        executor
            .resources
            .repos()
            .training_plans
            .get_active_plan(&tenant_id, &user_id.to_string(), Some("endurance-coach"))
            .await?
            .is_none(),
        "nothing is written"
    );
    Ok(())
}

#[tokio::test]
async fn a_plan_saved_without_running_the_rule_still_reports_its_flavour() -> Result<()> {
    let executor = create_executor().await?;
    let (user_id, tenant_id) = create_test_user(&executor).await?;
    let (events, guard) = setup_capture();
    let saved = executor
        .execute_tool(request(
            "save_training_plan",
            plan_payload(&json!({
                "id": "polarized-classic",
                "selected_by": "coach",
                "override_reason": "house style"
            })),
            user_id,
            &tenant_id,
        ))
        .await?;
    let captured = events.lock().expect("capture lock").clone();
    drop(guard);
    assert!(saved.success, "save failed: {:?}", saved.error);

    let saved_events = events_named(&captured, "training_plan.vision_saved");
    assert_eq!(saved_events.len(), 1);
    assert_eq!(
        saved_events[0].fields.get("confidence").map(String::as_str),
        Some("unknown")
    );
    let overridden = events_named(&captured, "training_plan.flavour_overridden");
    assert_eq!(
        overridden.len(),
        1,
        "a coach choice with no verdict is still an override"
    );
    assert_eq!(
        overridden[0].fields.get("rule_top").map(String::as_str),
        Some("unknown")
    );
    Ok(())
}
