// ABOUTME: Whose data a coach reply may show — the roster tools that mark a turn, and the coach's roster scope
// ABOUTME: The platform half of the persona style stage; dravr-colleur runs the checks over what this resolves
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Roster reads for the persona style stage.
//!
//! dravr-colleur checks a coach reply's athlete citations against a
//! [`RosterScope`], and its attribution rule fires only when a coached
//! athlete's data reached the turn. Both facts are the platform's: which
//! tools read a roster, and which athletes a coach coaches.

use dravr_colleur::RosterScope;
use pierre_core::errors::AppResult;
use pierre_database::repositories::CoachingGroupRepository;
use uuid::Uuid;

use super::peer_grounding::PEER_FETCH_TOOL;

/// Tools after which a data block in a coach's reply is about a roster athlete rather than the
/// coach, so it must say whose it is.
///
/// Two of them return a coached athlete's sessions: the roster overview and the consent-gated
/// member fetch. The coaching-platform roster returns no session: it names the Dravr members on the
/// coach's platform roster with where each stands with the coach's groups, and counts every other
/// athlete by state and access, so a block built from it still lists athletes.
///
/// All three read through the caller's own coach standing, which is why the
/// server's tool-data table declares them as the caller's own reads; this
/// list says whose data the reply then shows, a different fact.
///
/// The member fetch is also the name the pipeline records when it fetches a
/// named peer itself ([`super::peer_grounding`],
/// [`super::capability_subject`]), so `tools_called` carries it whichever
/// side made the call.
const ROSTER_READ_TOOLS: &[&str] = &[
    "get_roster_overview",
    "get_provider_roster",
    PEER_FETCH_TOOL,
];

/// `true` when the turn read a coached athlete's data, judged from the names
/// of the tools that ran.
#[must_use]
pub(crate) fn turn_read_roster_data(tools_called: &[String]) -> bool {
    tools_called
        .iter()
        .any(|tool| ROSTER_READ_TOOLS.contains(&tool.as_str()))
}

/// The roster a human coach's reply may cite: every live member of every
/// active group whose `coach_user_id` is `coach_user_id`.
///
/// Groups span tenants and the coach attachment is the key, so no tenant
/// narrows the lookup. A user who coaches no group gets an empty scope.
///
/// # Errors
/// Returns the repository's error when the member lookup fails.
pub(crate) async fn coach_roster_scope(
    groups: &dyn CoachingGroupRepository,
    coach_user_id: Uuid,
) -> AppResult<RosterScope> {
    let athletes = groups.list_athletes_coached_by(coach_user_id).await?;
    Ok(RosterScope::from_athlete_ids(
        athletes.iter().map(Uuid::to_string),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::groups::UpdateGroupRequest;
    use pierre_core::models::TenantId;
    use pierre_database::backends::factory::Database;
    use pierre_database::RepositoryRegistry;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::delegation::{
        add_group_member, create_coached_group, create_group_agent,
    };
    use pierre_test_support::garmin::create_athlete;

    fn called(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_roster_read_marks_the_turn() {
        for tools in [
            called(&["get_roster_overview"]),
            called(&["get_provider_roster"]),
            called(&["get_activities", PEER_FETCH_TOOL]),
        ] {
            assert!(
                turn_read_roster_data(&tools),
                "bug: {tools:?} read a coached athlete's data"
            );
        }
    }

    #[test]
    fn the_callers_own_reads_do_not() {
        for tools in [
            called(&[]),
            called(&["get_activities", "discover_routes", "get_training_plan"]),
        ] {
            assert!(
                !turn_read_roster_data(&tools),
                "bug: {tools:?} are the caller's own reads, not a roster read"
            );
        }
    }

    /// The names are matched as strings, so a renamed tool would silently
    /// switch the attribution rule off; pin them to the tools' own definitions.
    #[cfg(feature = "tools-groups")]
    #[test]
    fn roster_read_tools_are_the_registered_names() {
        use dravr_tronc::mcp::tool::McpTool;
        use pierre_tool_runtime::implementations::group_roster::GetRosterOverviewTool;
        use pierre_tool_runtime::implementations::groups::GetGroupMemberActivitiesTool;
        use pierre_tool_runtime::implementations::provider_roster::GetProviderRosterTool;

        let registered = [
            GetRosterOverviewTool.definition().name,
            GetProviderRosterTool.definition().name,
            GetGroupMemberActivitiesTool.definition().name,
        ];
        for name in ROSTER_READ_TOOLS {
            assert!(
                registered.iter().any(|r| r == name),
                "{name} is not a registered roster tool: {registered:?}"
            );
        }
    }

    /// The lowercased last four characters of a uuid: the identity a citation
    /// carries and `RosterScope::allows` compares.
    fn last4(id: Uuid) -> String {
        let text = id.simple().to_string();
        text[text.len() - 4..].to_owned()
    }

    /// A coach of three groups: A (active; m1 live, m2 left), B (archived; m3
    /// still a live member row), C (active; m1 again). Plus a user who coaches
    /// nothing.
    struct Seeded {
        db: Database,
        tenant: TenantId,
        coach: Uuid,
        stranger: Uuid,
        group_a: Uuid,
        m1: Uuid,
        m2: Uuid,
        m3: Uuid,
    }

    async fn user(repos: &RepositoryRegistry) -> Uuid {
        create_athlete(repos).await.unwrap().0
    }

    async fn seeded() -> Seeded {
        // Runs on whichever driver the test factory selects.
        let db = create_test_db().await.unwrap();
        let repos = db.repositories();
        let (owner, tenant) = create_athlete(repos).await.unwrap();
        let coach = user(repos).await;
        let m1 = user(repos).await;
        let m2 = user(repos).await;
        let m3 = user(repos).await;
        let stranger = user(repos).await;

        let agent_id = create_group_agent(repos, owner, tenant).await.unwrap();
        let mut groups = Vec::new();
        for name in ["A", "B", "C"] {
            groups.push(
                create_coached_group(repos, owner, tenant, &agent_id, name, coach)
                    .await
                    .unwrap(),
            );
        }
        let (group_a, group_b, group_c) = (groups[0], groups[1], groups[2]);

        add_group_member(repos, group_a, m1, tenant).await.unwrap();
        add_group_member(repos, group_a, m2, tenant).await.unwrap();
        add_group_member(repos, group_b, m3, tenant).await.unwrap();
        add_group_member(repos, group_c, m1, tenant).await.unwrap();

        assert!(repos
            .groups
            .remove_member(&group_a.to_string(), m2)
            .await
            .unwrap());
        // Archived by flag alone, so m3's membership row stays live: the group's
        // own `is_active` is what must exclude it.
        repos
            .groups
            .update_group(
                &group_b.to_string(),
                tenant,
                &UpdateGroupRequest {
                    name: None,
                    description: None,
                    agent_id: None,
                    peer_data_sharing: None,
                    respond_mode: None,
                    digest_mode: None,
                    is_active: Some(false),
                },
            )
            .await
            .unwrap()
            .expect("group B is updated under its tenant");
        assert!(repos
            .groups
            .get_member(&group_b.to_string(), m3)
            .await
            .unwrap()
            .is_some_and(|m| m.left_at.is_none()));

        Seeded {
            db,
            tenant,
            coach,
            stranger,
            group_a,
            m1,
            m2,
            m3,
        }
    }

    #[tokio::test]
    async fn the_roster_is_the_live_members_of_active_coached_groups() {
        let s = seeded().await;
        let repos = s.db.repositories();

        assert_eq!(
            repos
                .groups
                .list_athletes_coached_by(s.coach)
                .await
                .unwrap(),
            vec![s.m1],
            "m1 once though a member of two coached groups; m2 left; m3's group is archived"
        );

        let scope = coach_roster_scope(repos.groups.as_ref(), s.coach)
            .await
            .unwrap();
        assert!(scope.allows(&last4(s.m1)));
        assert!(
            scope.allows(&last4(s.m1).to_uppercase()),
            "a citation's case does not matter"
        );
        assert!(!scope.allows(&last4(s.m2)), "m2 left group A");
        assert!(!scope.allows(&last4(s.m3)), "group B is archived");
        assert!(!scope.allows(&last4(s.stranger)));
        assert!(!scope.is_empty());
    }

    #[tokio::test]
    async fn a_user_who_coaches_no_group_has_an_empty_roster() {
        let s = seeded().await;
        let repos = s.db.repositories();

        let got = repos
            .groups
            .list_athletes_coached_by(s.stranger)
            .await
            .unwrap();
        assert!(got.is_empty(), "expected empty, got {got:?}");
        assert!(coach_roster_scope(repos.groups.as_ref(), s.stranger)
            .await
            .unwrap()
            .is_empty());
        assert!(
            coach_roster_scope(repos.groups.as_ref(), s.m1)
                .await
                .unwrap()
                .is_empty(),
            "being a member of a group is not coaching it"
        );
    }

    #[tokio::test]
    async fn detaching_the_coach_takes_the_group_off_their_roster() {
        let s = seeded().await;
        let repos = s.db.repositories();

        assert!(repos
            .groups
            .set_group_coach_user(&s.group_a.to_string(), None, s.tenant)
            .await
            .unwrap());

        // m1 is still coached through group C, so only A's detachment is visible
        // once C goes too.
        assert_eq!(
            repos
                .groups
                .list_athletes_coached_by(s.coach)
                .await
                .unwrap(),
            vec![s.m1]
        );
        let groups = repos.groups.list_groups_coached_by(s.coach).await.unwrap();
        assert_eq!(groups.len(), 1, "only group C is still coached");
        assert!(repos
            .groups
            .set_group_coach_user(&groups[0].id.to_string(), None, s.tenant)
            .await
            .unwrap());

        let got = repos
            .groups
            .list_athletes_coached_by(s.coach)
            .await
            .unwrap();
        assert!(got.is_empty(), "expected empty, got {got:?}");
        assert!(coach_roster_scope(repos.groups.as_ref(), s.coach)
            .await
            .unwrap()
            .is_empty());
    }
}
