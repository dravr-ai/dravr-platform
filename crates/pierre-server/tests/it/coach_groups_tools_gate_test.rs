// ABOUTME: The groups tools' coach reads are gated as every coach read of a coached athlete is: one answer, one refusal
// ABOUTME: A coach the athlete's consent or tenancy refuses is refused by the groups tools exactly as by a scoped read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1425. A coach's read of a named athlete used to go through two
//! gates: the `groups` tools asked `member_read_allowed`, the subject scope
//! (carnet#1387) asked the coaching gates and the data class. They now ask
//! the same answer, so a coach the athlete's consent or tenancy refuses gets
//! the same `reason` and the same words from either, and a turn replays rows
//! about the athlete under that same answer.
//!
//! Every groups tool here reads the training class: activities, the roster's
//! training snapshot, the plan's status. None reads wellness.

#[cfg(feature = "tools-groups")]
mod groups_tools_gate {
    use chrono::Utc;
    use pierre_core::models::{ConnectionType, Tenant, TenantId};
    use pierre_services::member_reads::readable_members;
    use pierre_tool_runtime::protocols::UniversalResponse;
    use serde_json::{json, Value};

    use crate::coach_athlete_reads_test::{fixture, mentions, refused_reason, Fixture};

    /// The roster name the groups tools are asked about: the fixture's athlete.
    const MEMBER: &str = "Mira";

    impl Fixture {
        /// Let the coach review a roster at all (`manages_roster`), and give the
        /// athlete a training source, so the roster lists them as a sharer and
        /// gates their read rather than listing them as having nothing to share.
        async fn with_roster(&self) {
            self.repos()
                .users
                .set_manages_roster(self.coach.id, true)
                .await
                .unwrap();
            self.repos()
                .provider_connections
                .register_connection(
                    self.athlete.id,
                    self.athlete.tenant,
                    "strava",
                    &ConnectionType::OAuth,
                    None,
                )
                .await
                .unwrap();
        }

        /// What a groups tool answers the coach with in their own conversation.
        async fn groups_tool(&self, tool: &str, params: Value) -> UniversalResponse {
            self.call(&self.own_executor(), self.coach, tool, params)
                .await
        }

        /// The athlete belongs to a second tenant too.
        async fn give_the_athlete_a_second_tenant(&self) {
            let now = Utc::now();
            let second = TenantId::generate();
            self.repos()
                .tenants
                .create(&Tenant {
                    id: second,
                    name: "Second home".to_owned(),
                    slug: format!("second-{second}"),
                    domain: None,
                    plan: "professional".to_owned(),
                    owner_user_id: self.athlete.id,
                    created_at: now,
                    updated_at: now,
                })
                .await
                .unwrap();
        }

        /// Whether a turn of the coach's in the group replays rows about the
        /// athlete.
        async fn replays_the_athlete(&self) -> bool {
            readable_members(
                self.repos(),
                self.coach.id,
                Some(self.group),
                &[self.athlete.id],
            )
            .await
            .unwrap()
                == [self.athlete.id]
        }
    }

    /// A refusal's `reason` and words, as the caller reads them.
    fn refusal(response: &UniversalResponse) -> (String, String) {
        (
            refused_reason(response),
            response.error.clone().expect("a refusal carries its words"),
        )
    }

    /// The roster's row for the athlete.
    fn roster_row(payload: &Value) -> Value {
        payload["athletes"]
            .as_array()
            .expect("the roster lists athletes")
            .iter()
            .find(|row| {
                row["name"]
                    .as_str()
                    .is_some_and(|name| name.contains(MEMBER))
            })
            .cloned()
            .unwrap_or_else(|| panic!("the athlete is on the roster: {payload}"))
    }

    #[tokio::test]
    async fn a_revoked_coach_consent_refuses_the_groups_tools_as_it_refuses_a_scoped_read() {
        let f = fixture().await;
        f.with_roster().await;
        assert!(
            f.replays_the_athlete().await,
            "test setup bug: a sharing athlete's rows replay into their coach's turn"
        );
        f.repos()
            .groups
            .update_coach_sharing_consent(&f.group.to_string(), f.athlete.id, false)
            .await
            .unwrap();

        let in_thread = refusal(&f.in_thread("get_activities", json!({})).await);
        let over_mcp = refusal(
            &f.call(
                &f.mcp_executor(),
                f.coach,
                "get_activities",
                json!({ "athlete": MEMBER }),
            )
            .await,
        );
        assert_eq!(
            in_thread.0, "coach_sharing_revoked",
            "test setup bug: the scoped read refuses a revoked consent"
        );
        assert_eq!(
            in_thread, over_mcp,
            "test setup bug: both scoped paths word the refusal alike"
        );

        let activities = f
            .groups_tool("get_group_member_activities", json!({ "member": MEMBER }))
            .await;
        assert_eq!(
            refusal(&activities),
            in_thread,
            "bug: the groups tool refuses a revoked consent as a scoped read does"
        );
        assert!(
            !mentions(activities.result.as_ref().unwrap(), "athlete-run-"),
            "bug: nothing of the athlete's is read"
        );

        let plan = f
            .groups_tool("get_group_member_plan_status", json!({ "member": MEMBER }))
            .await;
        assert_eq!(
            plan.result.as_ref().unwrap()["reason"],
            "coach_sharing_revoked",
            "bug: the plan status refuses a revoked consent with the scoped read's reason: {:?}",
            plan.result
        );
        assert_eq!(
            plan.error.as_deref(),
            Some(in_thread.1.as_str()),
            "bug: and in its words"
        );

        let roster = f.groups_tool("get_roster_overview", json!({})).await;
        let row = roster_row(roster.result.as_ref().unwrap());
        assert_eq!(row["status"], "no_coach_consent", "{row}");
        assert!(
            row.get("training").is_none(),
            "bug: the roster reads no training of an athlete who stopped sharing: {row}"
        );

        assert!(
            !f.replays_the_athlete().await,
            "bug: a turn replays no row about an athlete who stopped sharing"
        );
    }

    #[tokio::test]
    async fn an_athlete_in_two_tenants_is_refused_by_the_groups_tools_as_by_a_scoped_read() {
        let f = fixture().await;
        f.with_roster().await;
        f.give_the_athlete_a_second_tenant().await;

        let in_thread = refusal(&f.in_thread("get_activities", json!({})).await);
        assert_eq!(
            in_thread.0, "several_tenants",
            "test setup bug: the scoped read refuses an athlete of two tenants"
        );

        let activities = f
            .groups_tool("get_group_member_activities", json!({ "member": MEMBER }))
            .await;
        assert_eq!(
            refusal(&activities),
            in_thread,
            "bug: the groups tool refuses an athlete of two tenants as a scoped read does, \
             where it used to read them"
        );

        let roster = f.groups_tool("get_roster_overview", json!({})).await;
        let row = roster_row(roster.result.as_ref().unwrap());
        assert!(
            row.get("training").is_none(),
            "bug: the roster reads no training a scoped read is refused: {row}"
        );
        assert_eq!(
            row["not_read"].as_str(),
            Some(in_thread.1.as_str()),
            "bug: the roster says why, in the scoped read's words: {row}"
        );

        let plan = f
            .groups_tool("get_group_member_plan_status", json!({ "member": MEMBER }))
            .await;
        assert_eq!(
            plan.result.as_ref().unwrap()["reason"],
            "several_tenants",
            "bug: the plan status refuses with the scoped read's reason: {:?}",
            plan.result
        );

        assert!(
            !f.replays_the_athlete().await,
            "bug: a turn replays no row about an athlete its coach may no longer read"
        );
    }

    #[tokio::test]
    async fn a_sharing_athlete_of_one_tenant_is_read_by_the_roster_under_the_same_answer() {
        let f = fixture().await;
        f.with_roster().await;

        let roster = f.groups_tool("get_roster_overview", json!({})).await;
        let row = roster_row(roster.result.as_ref().unwrap());
        assert_eq!(row["status"], "shared", "{row}");
        assert!(
            row.get("not_read").is_none(),
            "bug: the gate lets the coach read an athlete who shares: {row}"
        );
        assert!(
            row.get("training").is_some(),
            "bug: the roster reads the training of an athlete the gate lets the coach read: {row}"
        );
        assert!(
            f.replays_the_athlete().await,
            "bug: a sharing athlete's rows replay into their coach's turn"
        );
    }
}
