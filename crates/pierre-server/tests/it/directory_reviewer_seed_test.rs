// ABOUTME: `pierre-cli seed directory-reviewer` leaves the coach the connector-directory reviewers sign in with, listed as a coach
// ABOUTME: Read back over /mcp as the reviewers' Claude reads it: one member named, one counted, one named with data withheld

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The directory reviewers (carnet#1511, plan step 16, Q18) sign in as a
//! human coach: a plain account with coach access, one group, three members.
//! The listing they review is the coach's, never an admin's, and the roster
//! shows each opt-in state. A seeder whose accounts drift from what the
//! server computes would hand the reviewers a listing that differs from the
//! one submitted, so the seeded account is read here over `/mcp`, the way
//! their connection reads it.

#[cfg(feature = "tools-groups")]
mod directory_reviewer_seed_tests {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_core::constants::oauth::providers::{INTERVALS_ICU, STRAVA};
    use pierre_core::errors::AppError;
    use pierre_core::models::groups::{GroupInvite, GroupInviteKind};
    use pierre_core::models::{
        Activity, ActivityBuilder, AddMessageParams, CarriedRead, SportType, Tenant, TenantId,
        UserOAuthToken, UserStatus,
    };
    use pierre_core::transport::TransportPolicy;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::repositories::import_tombstones::{email_identity_hash, TombstoneKind};
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_seeders::directory_reviewer::{run, SeedArgs};
    use pierre_services::barrage_keys::memory_records;
    use pierre_services::barrage_live_store::live_store;
    use pierre_services::coach_roster::{roster_access, RosterAccess};
    use pierre_services::live_activities::{LiveActivityStore, WriteOrigin};
    use pierre_tool_runtime::implementations::group_roster::KEPT_IN_DRAVR;
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::common::{self, create_test_server_resources};
    use pierre_test_support::account::delete_account;
    use pierre_test_support::delegation::{
        add_group_member, create_coached_group, create_coachless_group, create_group_agent,
    };

    const COACH: &str = "reviewer@directory-seed.test";

    /// The group the reviewers lead, as the seeder names it.
    const GROUP_NAME: &str = "Directory Review Squad";

    /// The members the seeder adds: `(member number, display name, allowed
    /// the coach's assistant, the one source they sync from)`.
    const MEMBERS: [(usize, &str, bool, &str); 3] = [
        (1, "Ava Lambert", true, INTERVALS_ICU),
        (2, "Ben Okafor", false, INTERVALS_ICU),
        (3, "Cleo Strand", true, STRAVA),
    ];

    /// The sessions the seeder keeps for each member.
    const SESSIONS: u64 = 7;

    /// A test-only password, long enough for the seeder.
    const PASSWORD: &str = "test-only-reviewer-passphrase";

    /// The three coach tools (carnet#1511).
    const COACH_TOOLS: [&str; 3] = [
        "get_group_member_activities",
        "get_group_member_plan_status",
        "get_roster_overview",
    ];

    fn args(password: &str) -> SeedArgs {
        args_for(COACH, password)
    }

    fn args_for(coach_email: &str, password: &str) -> SeedArgs {
        SeedArgs {
            coach_email: coach_email.to_owned(),
            password: SecretString::from(password.to_owned()),
        }
    }

    /// The email of member `number` of the coach with id `coach_id`: derived
    /// from the coach's account, so no two coaches share a member.
    fn member_email_of(coach_id: Uuid, number: usize) -> String {
        format!(
            "directory-reviewer-{}-member-{number}@demo.pierre.dev",
            coach_id.simple()
        )
    }

    /// The email of member `number` of the seeded reviewer.
    async fn member_email(res: &Arc<ServerContext>, number: usize) -> String {
        member_email_of(seeded(res, COACH).await.0, number)
    }

    /// The activity store the seeder keeps its sessions in: barrage, over a
    /// record store of the test's own, projecting into `res`' cache.
    fn store(res: &Arc<ServerContext>) -> Arc<dyn LiveActivityStore> {
        live_store(Arc::new(memory_records()), None, &res.common.repos)
    }

    /// A seeded account's id and tenant.
    async fn seeded(res: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId) {
        let repos = &res.common.repos;
        let user = repos
            .seeder
            .seed_find_user_by_email(email)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{email} is seeded"));
        let tenant = repos
            .seeder
            .seed_get_user_tenant(user.id)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{email} has a tenant"));
        (
            user.id,
            TenantId::from_uuid(Uuid::parse_str(&tenant).unwrap()),
        )
    }

    async fn post_json(app: &Router, authorization: &str, body: Value) -> Value {
        let request = Request::post("/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("authorization", authorization)
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        serde_json::from_slice(&bytes).unwrap()
    }

    fn rpc(method: &str, params: &Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    }

    async fn mcp_call(app: &Router, grant: &str, name: &str, arguments: Value) -> Value {
        post_json(
            app,
            grant,
            rpc(
                "tools/call",
                &json!({ "name": name, "arguments": arguments }),
            ),
        )
        .await
    }

    #[tokio::test]
    async fn the_reviewer_is_a_coach_whose_roster_shows_every_opt_in_state() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();

        // A plain account with coach access an admin's grant gave, leading
        // one group, signing in with the password it was given.
        let (coach_id, coach_tenant) = seeded(&res, COACH).await;
        let coach = repos.users.get_global(coach_id).await.unwrap().unwrap();
        assert!(!coach.role.is_admin(), "never an admin");
        assert!(
            bcrypt::verify(PASSWORD, &coach.password_hash).unwrap(),
            "the reviewers sign in with the password the run was given"
        );
        assert!(
            repos
                .users
                .manages_roster_admin_grant(coach_id)
                .await
                .unwrap()
                .is_some(),
            "coach access is an admin's grant, which no reconciler takes back"
        );
        assert!(
            matches!(
                roster_access(repos, coach_id).await.unwrap(),
                RosterAccess::Granted(_)
            ),
            "the one coach rule grants the reviewer"
        );
        let groups = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
        assert_eq!(groups.len(), 1, "the reviewer leads one group: {groups:?}");
        assert_eq!(
            groups[0].name, GROUP_NAME,
            "the reviewer leads the seeded group"
        );

        // Each member in a tenant of their own.
        let mut tenants = BTreeSet::from([coach_tenant.as_uuid()]);
        for (number, _, _, _) in MEMBERS {
            let local = member_email(&res, number).await;
            let (member_id, tenant) = seeded(&res, &local).await;
            let member = repos.users.get_global(member_id).await.unwrap().unwrap();
            assert!(!member.role.is_admin(), "{local} is never an admin");
            assert!(
                !bcrypt::verify(PASSWORD, &member.password_hash).unwrap(),
                "a member never shares the reviewers' password"
            );
            assert!(
                tenants.insert(tenant.as_uuid()),
                "{local} has a tenant of their own"
            );
        }

        // The reviewers' Claude, on a read-only connection.
        let app = ProviderToolRouter::build_http_app(&res);
        let grant = format!(
            "Bearer {}",
            common::delegated_test_token_for_tenant(&res, coach_id, Some(coach_tenant.to_string()))
        );
        let listed = post_json(&app, &grant, rpc("tools/list", &json!({}))).await;
        let listed: BTreeSet<String> = listed["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("{listed}"))
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect();
        for tool in COACH_TOOLS {
            assert!(listed.contains(tool), "{tool}: {listed:?}");
        }
        let admin_tools: BTreeSet<String> = res
            .mcp
            .tool_registry
            .admin_tool_schemas()
            .into_iter()
            .map(|schema| schema.name)
            .collect();
        assert!(
            listed.is_disjoint(&admin_tools),
            "the directory's listing carries no admin tool: {:?}",
            listed.intersection(&admin_tools).collect::<Vec<_>>()
        );

        let roster = mcp_call(&app, &grant, "get_roster_overview", json!({})).await;
        assert_eq!(roster["result"]["isError"], false, "{roster}");
        let overview = &roster["result"]["structuredContent"];
        let athletes = overview["athletes"]
            .as_array()
            .unwrap_or_else(|| panic!("the athletes: {roster}"));
        let named = |name: &str| athletes.iter().find(|a| a["name"] == name).cloned();

        let ava = named("Ava Lambert").unwrap_or_else(|| panic!("{roster}"));
        assert!(ava.get("training").is_some(), "read with training: {ava}");
        assert!(
            named("Ben Okafor").is_none(),
            "a member who did not allow it is never named: {roster}"
        );
        assert!(
            roster["result"]["content"]
                .to_string()
                .contains("1 member has not allowed AI tools"),
            "and is counted: {roster}"
        );
        let cleo = named("Cleo Strand").unwrap_or_else(|| panic!("{roster}"));
        assert!(cleo.get("training").is_none(), "{cleo}");
        assert_eq!(cleo["not_read"], KEPT_IN_DRAVR, "{cleo}");

        // The coach's member tools read the member who allowed it from the
        // warmed cache, and never the one who did not.
        let ava = mcp_call(
            &app,
            &grant,
            "get_group_member_activities",
            json!({ "member": "Ava" }),
        )
        .await;
        assert_eq!(ava["result"]["isError"], false, "{ava}");
        let ben = mcp_call(
            &app,
            &grant,
            "get_group_member_activities",
            json!({ "member": "Ben" }),
        )
        .await;
        assert_eq!(ben["result"]["isError"], true, "{ben}");
        assert!(
            ben["result"]["content"].to_string().contains("no_match"),
            "{ben}"
        );

        // A rerun is a no-op.
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
    }

    /// The seeded state, whole and once: one group, the three members each
    /// sharing with the coach, the two who allowed it opted in, and each
    /// member's sessions kept once.
    async fn assert_fully_seeded(res: &Arc<ServerContext>, live: &dyn LiveActivityStore) {
        let repos = &res.common.repos;
        let (coach_id, _) = seeded(res, COACH).await;
        let groups = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
        assert_eq!(groups.len(), 1, "one group, never a second: {groups:?}");
        assert_eq!(
            groups[0].name, GROUP_NAME,
            "the one group is the seeded one"
        );
        let group_id = groups[0].id.to_string();
        let members = repos.groups.list_members(&group_id).await.unwrap();
        assert_eq!(
            members.len(),
            MEMBERS.len(),
            "each member joined once: {members:?}"
        );
        for (number, name, allowed, provider) in MEMBERS {
            let (member_id, tenant) = seeded(res, &member_email(res, number).await).await;
            let member = repos
                .groups
                .get_member(&group_id, member_id)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{name} is a live member"));
            assert!(member.coach_sharing_consent, "{name} shares with the coach");
            assert_eq!(
                member.coach_consents.assistant_read, allowed,
                "{name}'s opt-in"
            );
            assert_eq!(
                live.held(member_id, tenant, provider).await.unwrap(),
                SESSIONS,
                "{name}'s sessions are kept once"
            );
        }
    }

    /// A run that fails after the group exists is completed by the next
    /// one: the missing members, their shares, opt-ins and sessions are
    /// added, and nothing is seeded twice.
    #[tokio::test]
    async fn a_rerun_completes_a_half_seeded_group() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        // Every member after the first is refused at the join, so the first
        // run stops after the group and its first member are written.
        fault(&res, &MEMBERS_AFTER_THE_FIRST, true).await;
        run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("the second member's join is refused");
        fault(&res, &MEMBERS_AFTER_THE_FIRST, false).await;
        let (coach_id, _) = seeded(&res, COACH).await;
        let groups = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
        assert_eq!(groups.len(), 1, "the failed run created the group");
        let half = repos
            .groups
            .list_members(&groups[0].id.to_string())
            .await
            .unwrap();
        assert_eq!(half.len(), 1, "and only its first member: {half:?}");

        // The rest of a half-seeded state the rerun completes rather than
        // skips: the first member's share and opt-in gone, and the group
        // left without its coach, as a run that failed between creating the
        // group and attaching the coach leaves it.
        let group_id = groups[0].id.to_string();
        let (ava, _) = seeded(&res, &member_email(&res, 1).await).await;
        assert!(
            repos
                .groups
                .update_coach_sharing_consent(&group_id, ava, false)
                .await
                .unwrap(),
            "the fixture withdraws Ava's share"
        );
        repos
            .groups
            .set_coach_assistant_read(&group_id, ava, false, Timestamp::now())
            .await
            .unwrap()
            .unwrap();
        let (_, coach_tenant) = seeded(&res, COACH).await;
        repos
            .groups
            .set_group_coach_user(&group_id, None, coach_tenant)
            .await
            .unwrap();
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "the fixture leaves the group without its coach"
        );

        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
    }

    /// The agent the seeder makes for the group, as the seeder titles it.
    const AGENT_TITLE: &str = "Directory Review Squad Agent";

    /// The coach's own agents titled as the group's agent.
    async fn group_agents(res: &Arc<ServerContext>, coach_id: Uuid, tenant: TenantId) -> Vec<Uuid> {
        res.common
            .repos
            .agents
            .search(coach_id, tenant, AGENT_TITLE, None, Some(100), None)
            .await
            .unwrap()
            .into_iter()
            .filter(|agent| agent.title == AGENT_TITLE)
            .map(|agent| agent.id)
            .collect()
    }

    /// The insert of the reviewer's group: a run then dies after creating
    /// the group's agent and before the group.
    const REVIEWER_GROUP: Fault = Fault {
        name: "fault_directory_reviewer_group",
        table: "coaching_groups",
        when: "NEW.name = 'Directory Review Squad'",
    };

    /// A run that fails after creating the group's agent and before the
    /// group leaves that agent with no group: the next run creates the group
    /// with it, under the id the agent is derived to hold, and the coach
    /// owns one such agent, never a second.
    #[tokio::test]
    async fn a_rerun_after_an_orphaned_agent_reuses_it() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        fault(&res, &REVIEWER_GROUP, true).await;
        run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("the group's insert is refused");
        fault(&res, &REVIEWER_GROUP, false).await;
        let (coach_id, coach_tenant) = seeded(&res, COACH).await;
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "the failed run left no group"
        );
        let orphans = group_agents(&res, coach_id, coach_tenant).await;
        assert_eq!(orphans.len(), 1, "and the group's agent: {orphans:?}");
        let orphan = orphans[0];

        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
        assert_eq!(
            group_agents(&res, coach_id, coach_tenant).await,
            vec![orphan],
            "the orphaned agent is reused, never doubled"
        );
        let groups = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
        assert_eq!(
            groups[0].agent_id,
            orphan.to_string(),
            "the group runs the agent the failed run made"
        );

        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_eq!(
            group_agents(&res, coach_id, coach_tenant).await,
            vec![orphan],
            "a rerun over the seeded group makes no agent"
        );
    }

    /// An existing account at the coach email that signs in with the
    /// password, coaching no group yet: the account a run adopts. The
    /// coach's id and tenant.
    async fn bare_coach(res: &Arc<ServerContext>) -> (Uuid, TenantId) {
        let repos = &res.common.repos;
        let (coach_id, _user, _tenant) =
            common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                .await
                .unwrap();
        repos
            .users
            .update_password(coach_id, &bcrypt::hash(PASSWORD, 4).unwrap())
            .await
            .unwrap();
        seeded(res, COACH).await
    }

    /// Two runs racing each other end on what one run leaves: one group,
    /// coached by the reviewer, with one agent and the three members, each
    /// joined once. Both runs succeed, the coach created by one of them or
    /// already there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn two_concurrent_runs_seed_one_group_with_one_agent() {
        for iteration in 0..4 {
            let res = create_test_server_resources().await.unwrap();
            if iteration % 2 == 1 {
                let (coach_id, _user, _tenant) =
                    common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                        .await
                        .unwrap();
                res.common
                    .repos
                    .users
                    .update_password(coach_id, &bcrypt::hash(PASSWORD, 4).unwrap())
                    .await
                    .unwrap();
            }
            let live = store(&res);
            let racing = |res: Arc<ServerContext>, live: Arc<dyn LiveActivityStore>| {
                tokio::spawn(async move {
                    run(args(PASSWORD), &res.common.repos, live.as_ref())
                        .await
                        .map_err(|e| e.to_string())
                })
            };
            let first = racing(Arc::clone(&res), Arc::clone(&live));
            let second = racing(Arc::clone(&res), Arc::clone(&live));
            let (first, second) = (first.await.unwrap(), second.await.unwrap());
            assert!(
                first.is_ok() && second.is_ok(),
                "iteration {iteration}: both runs succeed: {first:?} {second:?}"
            );
            assert_fully_seeded(&res, live.as_ref()).await;
            let (coach_id, coach_tenant) = seeded(&res, COACH).await;
            assert!(
                res.common
                    .repos
                    .groups
                    .list_coachless_groups_owned_by(coach_id, coach_tenant)
                    .await
                    .unwrap()
                    .is_empty(),
                "iteration {iteration}: no second group left without its coach"
            );
            assert_eq!(
                group_agents(&res, coach_id, coach_tenant).await.len(),
                1,
                "iteration {iteration}: one agent"
            );
        }
    }

    /// An account that already exists at the coach email and signs in with
    /// another password is refused, never re-passworded and never adopted:
    /// the run writes nothing and never repeats the password it was given.
    #[tokio::test]
    async fn an_existing_account_with_another_password_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (user_id, _user, _tenant) =
            common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                .await
                .unwrap();
        let before = repos
            .users
            .get_global(user_id)
            .await
            .unwrap()
            .unwrap()
            .password_hash;
        let live = store(&res);
        let message = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("another password is refused")
            .to_string();
        assert!(
            message.contains("already exists") && message.contains("was not modified"),
            "{message}"
        );
        assert!(!message.contains(PASSWORD), "never the password: {message}");
        let after = repos.users.get_global(user_id).await.unwrap().unwrap();
        assert_eq!(after.password_hash, before, "the password is unchanged");
        assert!(
            repos
                .users
                .manages_roster_admin_grant(user_id)
                .await
                .unwrap()
                .is_none(),
            "no coach access is granted"
        );
        assert!(
            repos
                .groups
                .list_groups_coached_by(user_id)
                .await
                .unwrap()
                .is_empty(),
            "a refused run leads no group"
        );

        // The seeded reviewer is refused alike when the password passed is
        // not the one it was seeded with.
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let other = "another-reviewer-passphrase";
        let message = run(args(other), repos, live.as_ref())
            .await
            .expect_err("a rerun never changes the password")
            .to_string();
        assert!(!message.contains(other), "never the password: {message}");
        let (coach_id, _) = seeded(&res, COACH).await;
        let coach = repos.users.get_global(coach_id).await.unwrap().unwrap();
        assert!(
            bcrypt::verify(PASSWORD, &coach.password_hash).unwrap(),
            "the refused rerun keeps the seeded password"
        );
        assert_fully_seeded(&res, live.as_ref()).await;
    }

    #[tokio::test]
    async fn a_short_password_is_refused_before_anything_is_written() {
        let res = create_test_server_resources().await.unwrap();
        let live = store(&res);
        let short = "short-pass";
        let refused = run(args(short), &res.common.repos, live.as_ref()).await;
        let message = refused
            .expect_err("a short password is refused")
            .to_string();
        assert!(
            !message.contains(short),
            "the refusal never repeats the password: {message}"
        );
        assert!(
            res.common
                .repos
                .seeder
                .seed_find_user_by_email(COACH)
                .await
                .unwrap()
                .is_none(),
            "nothing is written"
        );
    }

    #[tokio::test]
    async fn an_admin_is_never_made_the_reviewer() {
        let res = create_test_server_resources().await.unwrap();
        let (admin_id, _user, _tenant) =
            common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                .await
                .unwrap();
        res.common
            .repos
            .users
            .set_admin_status(admin_id, true)
            .await
            .unwrap();
        let live = store(&res);
        let refused = run(args(PASSWORD), &res.common.repos, live.as_ref())
            .await
            .expect_err("an admin is listed every tool");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("is an admin") && said.contains("--coach-email") && !said.contains(COACH),
            "the refusal names the remedy, the email masked: {said}"
        );
        assert!(
            res.common
                .repos
                .groups
                .list_groups_coached_by(admin_id)
                .await
                .unwrap()
                .is_empty(),
            "an admin refused as coach is given no group"
        );
    }

    /// A value a test statement binds: an id its column stores as `uuid` on
    /// Postgres and as text on `SQLite`, or text on both.
    enum Bound {
        Id(Uuid),
        Text(String),
    }

    /// Run `sql` on the test's own database, whichever backend it is, with
    /// `binds` as its `$1`.. parameters.
    async fn execute(res: &Arc<ServerContext>, sql: &str, binds: &[Bound]) {
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                let mut query = sqlx::query(sql);
                for bound in binds {
                    query = match bound {
                        Bound::Id(id) => query.bind(id.to_string()),
                        Bound::Text(text) => query.bind(text.clone()),
                    };
                }
                query.execute(db.pool()).await.unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                let mut query = sqlx::query(sql);
                for bound in binds {
                    query = match bound {
                        Bound::Id(id) => query.bind(*id),
                        Bound::Text(text) => query.bind(text.clone()),
                    };
                }
                query.execute(db.pool()).await.unwrap();
            }
        }
    }

    /// A write the test makes fail: a trigger `name` that refuses an insert
    /// into `table` whenever `when` holds of the new row (`NEW`).
    struct Fault {
        name: &'static str,
        table: &'static str,
        when: &'static str,
    }

    /// The insert of the coach's tenant: a run then dies inside the coach's
    /// account creation, after its user row.
    const COACH_TENANT: Fault = Fault {
        name: "fault_directory_reviewer_tenant",
        table: "tenants",
        when: "NEW.name = 'Directory Reviewer''s Workspace'",
    };

    /// The join of every member after a group's first: a run then stops
    /// after the group and its first member are written.
    const MEMBERS_AFTER_THE_FIRST: Fault = Fault {
        name: "fault_directory_reviewer_members",
        table: "coaching_group_members",
        when: "(SELECT COUNT(*) FROM coaching_group_members \
               WHERE group_id = NEW.group_id) >= 1",
    };

    /// Make `fault`'s write fail, or stop failing it.
    async fn fault(res: &Arc<ServerContext>, fault: &Fault, refused: bool) {
        let Fault { name, table, when } = fault;
        match (res.agent.database.backend(), refused) {
            (DatabaseBackend::SQLite(_), true) => {
                execute(
                    res,
                    &format!(
                        "CREATE TRIGGER {name} BEFORE INSERT ON {table} WHEN {when} \
                         BEGIN SELECT RAISE(ABORT, 'refused by the test'); END"
                    ),
                    &[],
                )
                .await;
            }
            (DatabaseBackend::SQLite(_), false) => {
                execute(res, &format!("DROP TRIGGER {name}"), &[]).await;
            }
            #[cfg(feature = "postgresql")]
            (DatabaseBackend::PostgreSQL(_), true) => {
                // A Postgres trigger's WHEN takes no subquery: the function
                // tests the row instead.
                execute(
                    res,
                    &format!(
                        "CREATE OR REPLACE FUNCTION {name}() RETURNS trigger LANGUAGE plpgsql \
                         AS $$ BEGIN IF {when} THEN RAISE EXCEPTION 'refused by the test'; \
                         END IF; RETURN NEW; END $$"
                    ),
                    &[],
                )
                .await;
                execute(
                    res,
                    &format!("DROP TRIGGER IF EXISTS {name} ON {table}"),
                    &[],
                )
                .await;
                execute(
                    res,
                    &format!(
                        "CREATE TRIGGER {name} BEFORE INSERT ON {table} FOR EACH ROW \
                         EXECUTE FUNCTION {name}()"
                    ),
                    &[],
                )
                .await;
            }
            #[cfg(feature = "postgresql")]
            (DatabaseBackend::PostgreSQL(_), false) => {
                execute(res, &format!("DROP TRIGGER {name} ON {table}"), &[]).await;
            }
        }
    }

    /// A run that dies inside the coach's account creation, after its user
    /// row and before its tenant, leaves no account behind: the next run
    /// creates it whole and seeds everything, where an account left without
    /// its tenant would refuse every rerun.
    #[tokio::test]
    async fn a_run_that_dies_creating_an_account_leaves_none_behind() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        fault(&res, &COACH_TENANT, true).await;
        run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("the coach's tenant is refused");
        assert!(
            repos
                .seeder
                .seed_check_user_exists(COACH)
                .await
                .unwrap()
                .is_none(),
            "the account is rolled back with its tenant, never left without one"
        );
        fault(&res, &COACH_TENANT, false).await;
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
    }

    /// An account the seeder finds without its tenant is refused with its
    /// email masked, as every other seeder message names it.
    #[tokio::test]
    async fn an_account_without_its_tenant_is_refused_with_its_email_masked() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let cleo = member_email(&res, 3).await;
        let (member, _) = seeded(&res, &cleo).await;
        execute(
            &res,
            "UPDATE users SET tenant_id = NULL WHERE id = $1",
            &[Bound::Id(member)],
        )
        .await;
        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("an account without its tenant is refused");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("has no tenant_id") && said.contains("--coach-email"),
            "the refusal says why and names the remedy: {said}"
        );
        assert!(!said.contains(&cleo), "the email is masked: {said}");
    }

    /// A member's email may already hold an account the seeder did not
    /// create for this coach: it is refused, unmodified and named masked,
    /// before any member is written to or any group made.
    #[tokio::test]
    async fn an_account_at_a_members_email_the_seeder_did_not_create_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        // The reviewer exists already, signing in with the password passed.
        let (coach_id, _user, _tenant) =
            common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                .await
                .unwrap();
        repos
            .users
            .update_password(
                coach_id,
                &bcrypt::hash(PASSWORD, bcrypt::DEFAULT_COST).unwrap(),
            )
            .await
            .unwrap();
        let theirs = member_email_of(coach_id, 1);
        let (stranger, _user, _tenant) =
            common::create_test_user_with_plan(&res.agent.database, &theirs, "professional")
                .await
                .unwrap();
        let live = store(&res);
        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("an account the seeder did not create is never adopted");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("did not create") && !said.contains(&theirs),
            "refused, the email masked: {said}"
        );
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "no group is made, so no one joins one"
        );
        assert!(
            repos
                .provider_connections
                .get_for_user(stranger, None)
                .await
                .unwrap()
                .is_empty(),
            "the account gains no source"
        );
    }

    /// Two coach emails seed two reviewers whose members are their own:
    /// never the same accounts enrolled under a second coach.
    #[tokio::test]
    async fn two_coach_emails_never_enrol_the_same_members() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        let other = "second-reviewer@directory-seed.test";
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        run(args_for(other, PASSWORD), repos, live.as_ref())
            .await
            .unwrap();
        let mut seen = BTreeSet::new();
        for coach in [COACH, other] {
            let (coach_id, _) = seeded(&res, coach).await;
            let groups = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
            assert_eq!(groups.len(), 1, "{coach} leads one group: {groups:?}");
            let members = repos
                .groups
                .list_members(&groups[0].id.to_string())
                .await
                .unwrap();
            assert_eq!(members.len(), MEMBERS.len(), "{coach}'s members");
            for member in members {
                assert!(
                    seen.insert(member.user_id),
                    "a member of one reviewer is never enrolled under another: {}",
                    member.user_id
                );
            }
        }
    }

    /// A member who left the group is never added back: a rerun refuses
    /// with the reason, the email masked, before it writes anything — not
    /// the coach's access, nor any other member's share.
    #[tokio::test]
    async fn a_member_who_left_is_refused_before_anything_is_written() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (coach_id, _) = seeded(&res, COACH).await;
        let group = repos.groups.list_groups_coached_by(coach_id).await.unwrap()[0].id;
        let group_id = group.to_string();
        let ava = member_email(&res, 1).await;
        let (ava_id, _) = seeded(&res, &ava).await;
        let (cleo, _) = seeded(&res, &member_email(&res, 3).await).await;
        // What a run would converge, taken back: the coach's access, and the
        // last member's share.
        repos
            .users
            .set_manages_roster_by_admin(coach_id, false, None)
            .await
            .unwrap();
        assert!(
            repos
                .users
                .manages_roster_admin_grant(coach_id)
                .await
                .unwrap()
                .is_none(),
            "the fixture takes the coach's access back"
        );
        assert!(
            repos
                .groups
                .update_coach_sharing_consent(&group_id, cleo, false)
                .await
                .unwrap(),
            "Cleo's coach-sharing consent is withdrawn"
        );
        assert!(
            repos.groups.remove_member(&group_id, ava_id).await.unwrap(),
            "the fixture has Ava leave"
        );
        for _ in 0..2 {
            let refused = run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect_err("a member who left is never added back");
            let said = format!("{refused} {refused:?}");
            assert!(
                said.contains("never added back") && !said.contains(&ava),
                "refused with the reason, the email masked: {said}"
            );
            assert!(
                repos
                    .users
                    .manages_roster_admin_grant(coach_id)
                    .await
                    .unwrap()
                    .is_none(),
                "the refused run gave the coach no access"
            );
            assert!(
                !repos
                    .groups
                    .get_member(&group_id, cleo)
                    .await
                    .unwrap()
                    .unwrap()
                    .coach_sharing_consent,
                "the refused run wrote no member's share"
            );
        }
    }

    /// A workspace may hold a member's slug with no account at the member's
    /// email: anyone signed in can create a workspace with a slug of their
    /// choosing. The run refuses before it writes anything, with the
    /// member's email masked and the remedy, rather than failing on the
    /// slug's unique key part way.
    #[tokio::test]
    async fn a_workspace_holding_a_members_slug_is_refused_before_anything_is_written() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, _user, _tenant) =
            common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                .await
                .unwrap();
        repos
            .users
            .update_password(coach_id, &bcrypt::hash(PASSWORD, 4).unwrap())
            .await
            .unwrap();
        let (squatter, _user, _tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "squatter@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let theirs = member_email_of(coach_id, 2);
        let slug = theirs.split('@').next().unwrap().to_owned();
        repos
            .tenants
            .create(&Tenant::new(
                "Squatted".to_owned(),
                slug,
                None,
                "starter".to_owned(),
                squatter,
            ))
            .await
            .unwrap();
        let live = store(&res);
        for _ in 0..2 {
            let refused = run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect_err("a slug someone else holds is refused");
            let said = format!("{refused} {refused:?}");
            assert!(
                said.contains("already holds the slug")
                    && said.contains("no account at that email owns it")
                    && !said.contains("did not create it")
                    && said.contains("--coach-email")
                    && !said.contains(&theirs)
                    && !said.contains("UNIQUE"),
                "refused with the reason and the remedy, the email masked: {said}"
            );
            assert!(
                repos
                    .seeder
                    .seed_check_user_exists(&member_email_of(coach_id, 1))
                    .await
                    .unwrap()
                    .is_none(),
                "no member is created"
            );
            assert!(
                repos
                    .groups
                    .list_groups_coached_by(coach_id)
                    .await
                    .unwrap()
                    .is_empty(),
                "no group is made"
            );
            assert!(
                repos
                    .users
                    .manages_roster_admin_grant(coach_id)
                    .await
                    .unwrap()
                    .is_none(),
                "the coach is given no access"
            );
        }
    }

    /// A seeded member whose account is deleted leaves its workspace behind
    /// with no owner, still holding the member's slug. A rerun reads the
    /// slug from the workspaces themselves, so it refuses in the preflight,
    /// masked and with the remedy, rather than dying on the slug's unique
    /// key; nothing is written.
    #[tokio::test]
    async fn an_ownerless_workspace_at_a_members_slug_is_refused_before_anything_is_written() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (coach_id, _) = seeded(&res, COACH).await;
        let theirs = member_email(&res, 2).await;
        let (member, _) = seeded(&res, &theirs).await;
        delete_account(repos, member, None).await.unwrap();
        assert!(
            repos
                .seeder
                .seed_check_user_exists(&theirs)
                .await
                .unwrap()
                .is_none(),
            "premise: the member's account is gone"
        );
        let groups = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
        assert_eq!(groups.len(), 1, "premise: the seeded group");
        let members_before = repos
            .groups
            .list_members(&groups[0].id.to_string())
            .await
            .unwrap()
            .len();

        for _ in 0..2 {
            let refused = run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect_err("an ownerless workspace at the member's slug is refused");
            let said = format!("{refused} {refused:?}");
            assert!(
                said.contains("already holds the slug")
                    && said.contains("no account at that email owns it")
                    && !said.contains("did not create it")
                    && said.contains("--coach-email")
                    && !said.contains(&theirs)
                    && !said.contains("UNIQUE"),
                "refused in the preflight with the reason and the remedy, the email masked: {said}"
            );
            assert!(
                repos
                    .seeder
                    .seed_check_user_exists(&theirs)
                    .await
                    .unwrap()
                    .is_none(),
                "the member is not created again"
            );
            let after = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
            assert_eq!(after.len(), 1, "no group is made");
            assert_eq!(
                repos
                    .groups
                    .list_members(&after[0].id.to_string())
                    .await
                    .unwrap()
                    .len(),
                members_before,
                "no member is added"
            );
        }
    }

    /// The opt-ins converge both ways: the member who did not allow the
    /// coach's assistant is taken back to not allowing it.
    #[tokio::test]
    async fn a_rerun_takes_back_an_opt_in_the_seed_does_not_hold() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (coach_id, _) = seeded(&res, COACH).await;
        let group = repos.groups.list_groups_coached_by(coach_id).await.unwrap()[0].id;
        let (ben, _) = seeded(&res, &member_email(&res, 2).await).await;
        repos
            .groups
            .set_coach_assistant_read(&group.to_string(), ben, true, Timestamp::now())
            .await
            .unwrap()
            .expect("Ben is a live member");
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
    }

    /// A password bcrypt would read only the start of is refused before
    /// anything is written, and never repeated.
    #[tokio::test]
    async fn a_password_longer_than_bcrypt_reads_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let live = store(&res);
        let long = format!("{}-tail", "L".repeat(72));
        let refused = run(args(&long), &res.common.repos, live.as_ref())
            .await
            .expect_err("bcrypt would ignore the tail");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("at most 71 bytes") && !said.contains(&long),
            "{said}"
        );
        assert!(
            res.common
                .repos
                .seeder
                .seed_check_user_exists(COACH)
                .await
                .unwrap()
                .is_none(),
            "nothing is written"
        );
    }
    /// The reviewer's group after a full seed, and a member's share in it.
    async fn seeded_group(res: &Arc<ServerContext>) -> (Uuid, Uuid) {
        let (coach_id, _) = seeded(res, COACH).await;
        let group = res
            .common
            .repos
            .groups
            .list_groups_coached_by(coach_id)
            .await
            .unwrap()[0]
            .id;
        (coach_id, group)
    }

    /// Ben's share with the coach in `group`, which every refused rerun
    /// below must leave withdrawn.
    async fn ben_shares(res: &Arc<ServerContext>, group: Uuid) -> bool {
        let (ben, _) = seeded(res, &member_email(res, 2).await).await;
        res.common
            .repos
            .groups
            .get_member(&group.to_string(), ben)
            .await
            .unwrap()
            .unwrap()
            .coach_sharing_consent
    }

    /// Run the seeder over a state it did not create, and assert the run
    /// is refused with `reason`, the coach's email masked, Ben's withdrawn
    /// share left as it was, on every rerun.
    async fn assert_refused(res: &Arc<ServerContext>, group: Uuid, reason: &str) {
        let repos = &res.common.repos;
        let live = store(res);
        for _ in 0..2 {
            let refused = run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect_err("a state this run did not create is refused");
            let said = format!("{refused} {refused:?}");
            assert!(
                said.contains(reason) && said.contains("--coach-email") && !said.contains(COACH),
                "refused with the reason and the remedy, the email masked: {said}"
            );
            assert!(
                !ben_shares(res, group).await,
                "the refused run re-shared no member with any coach"
            );
        }
    }

    /// Withdraw Ben's share in `group`, so a run that writes is seen to.
    async fn withdraw_ben(res: &Arc<ServerContext>, group: Uuid) {
        let (ben, _) = seeded(res, &member_email(res, 2).await).await;
        assert!(
            res.common
                .repos
                .groups
                .update_coach_sharing_consent(&group.to_string(), ben, false)
                .await
                .unwrap(),
            "Ben's coach-sharing consent is withdrawn"
        );
    }

    /// The reviewer's group, now coached by another account: the rerun
    /// refuses rather than re-share its members with a coach the reviewer
    /// is not.
    #[tokio::test]
    async fn a_group_now_coached_by_another_account_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        run(args(PASSWORD), &res.common.repos, store(&res).as_ref())
            .await
            .unwrap();
        let (_, group) = seeded_group(&res).await;
        withdraw_ben(&res, group).await;
        let (other, _) = common::create_test_user(&res.agent.database).await.unwrap();
        execute(
            &res,
            "UPDATE coaching_groups SET coach_user_id = $1 WHERE id = $2",
            &[Bound::Id(other), Bound::Id(group)],
        )
        .await;
        assert_refused(&res, group, "coached by another account").await;
    }

    /// The reviewer's group, archived before any member joined: the rerun
    /// never fills an inactive group.
    #[tokio::test]
    async fn an_archived_group_is_refused_not_filled() {
        let res = create_test_server_resources().await.unwrap();
        run(args(PASSWORD), &res.common.repos, store(&res).as_ref())
            .await
            .unwrap();
        let (_, group) = seeded_group(&res).await;
        withdraw_ben(&res, group).await;
        execute(
            &res,
            "UPDATE coaching_groups SET is_active = FALSE WHERE id = $1",
            &[Bound::Id(group)],
        )
        .await;
        assert_refused(&res, group, "archived").await;
    }

    /// The reviewer's group in a workspace that is not the coach's, as an
    /// admin's tenant move leaves it: the coach could not be given back.
    #[tokio::test]
    async fn a_group_in_another_workspace_than_the_coachs_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        run(args(PASSWORD), &res.common.repos, store(&res).as_ref())
            .await
            .unwrap();
        let (coach_id, group) = seeded_group(&res).await;
        withdraw_ben(&res, group).await;
        let other_tenant = TenantId::from_uuid(Uuid::new_v4());
        execute(
            &res,
            "UPDATE coaching_groups SET coach_user_id = NULL, tenant_id = $1 WHERE id = $2",
            &[Bound::Text(other_tenant.to_string()), Bound::Id(group)],
        )
        .await;
        assert!(
            res.common
                .repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "the group moved to another workspace is no longer coached by the coach"
        );
        assert_refused(&res, group, "another workspace").await;
    }

    /// A reviewer coach account an admin suspended, or one still pending,
    /// cannot sign in with any password: the rerun refuses it, the email
    /// masked, rather than report a reviewer the directory cannot reach, and
    /// leaves its status as it was.
    #[tokio::test]
    async fn an_inactive_coach_account_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (coach_id, _) = seeded(&res, COACH).await;
        for status in [UserStatus::Suspended, UserStatus::Pending] {
            repos
                .users
                .update_status(coach_id, status, None)
                .await
                .unwrap();
            let refused = run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect_err("an account that cannot sign in is never the reviewer");
            let said = format!("{refused} {refused:?}");
            assert!(
                said.contains("not active")
                    && said.contains("--coach-email")
                    && !said.contains(COACH),
                "refused with the reason and the remedy, the email masked: {said}"
            );
            assert_eq!(
                repos
                    .users
                    .get_global(coach_id)
                    .await
                    .unwrap()
                    .unwrap()
                    .user_status,
                status,
                "the refused run left the account's status alone"
            );
        }
    }

    /// The reviewer's group, now owned by another account: the rerun refuses
    /// rather than fill another owner's group and agent with its members.
    #[tokio::test]
    async fn a_group_another_account_owns_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        run(args(PASSWORD), &res.common.repos, store(&res).as_ref())
            .await
            .unwrap();
        let (_, group) = seeded_group(&res).await;
        withdraw_ben(&res, group).await;
        let (other, _) = common::create_test_user(&res.agent.database).await.unwrap();
        execute(
            &res,
            "UPDATE coaching_groups SET owner_id = $1 WHERE id = $2",
            &[Bound::Id(other), Bound::Id(group)],
        )
        .await;
        assert_refused(&res, group, "owned by another account").await;
    }

    /// A member's seeded session the member deleted stays deleted, and the
    /// rerun converges on the rest: it does not stop at that member, so a
    /// later member's opt-in is still taken back.
    #[tokio::test]
    async fn a_deleted_session_stays_deleted_and_the_rest_converges() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (ava, ava_tenant) = seeded(&res, &member_email(&res, 1).await).await;
        // Her newest seeded session, deleted; the rest she kept.
        let mut sessions = seeded_sessions(&res, ava, ava_tenant).await;
        assert_eq!(sessions.len(), 7, "Ava's seeded sessions");
        let deleted = sessions.remove(0);
        assert!(
            live.erase(ava, ava_tenant, INTERVALS_ICU, deleted.id())
                .await
                .unwrap(),
            "the store held Ava's session"
        );
        let kept_ids: Vec<String> = sessions.iter().map(|s| s.id().to_owned()).collect();
        let (coach_id, _) = seeded(&res, COACH).await;
        let group = repos.groups.list_groups_coached_by(coach_id).await.unwrap()[0].id;
        let (ben, _) = seeded(&res, &member_email(&res, 2).await).await;
        repos
            .groups
            .set_coach_assistant_read(&group.to_string(), ben, true, Timestamp::now())
            .await
            .unwrap()
            .expect("Ben is a live member");
        // Ava's sessions as she kept them: when each started.
        let kept_starts = || async {
            let mut starts = Vec::new();
            for id in &kept_ids {
                let kept = repos
                    .activity_cache
                    .get_cached_activity(ava, &ava_tenant, INTERVALS_ICU, id)
                    .await
                    .unwrap()
                    .unwrap_or_else(|| panic!("Ava kept {id}"));
                starts.push(kept.start_date());
            }
            starts
        };
        let before = kept_starts().await;

        for _ in 0..2 {
            run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect("a session the member deleted does not stop the run");
            assert_eq!(
                live.held(ava, ava_tenant, INTERVALS_ICU).await.unwrap(),
                SESSIONS - 1,
                "the deleted session is never brought back"
            );
            assert!(
                !repos
                    .groups
                    .get_member(&group.to_string(), ben)
                    .await
                    .unwrap()
                    .unwrap()
                    .coach_consents
                    .assistant_read,
                "the member after Ava converges: Ben's opt-in is taken back"
            );
            assert_eq!(
                kept_starts().await,
                before,
                "a rerun never rewrites or re-dates a session Ava kept"
            );
        }
    }

    /// An existing account adopted as the reviewer coach, which signs in
    /// with the password passed but works in a workspace another account
    /// owns (where an admin's approval puts it), is refused before anything
    /// is written, the email masked and the remedy named: the reviewer's
    /// group never lands in someone else's workspace.
    #[tokio::test]
    async fn an_adopted_coach_in_another_accounts_workspace_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, _, _) =
            common::create_test_user_with_plan(&res.agent.database, COACH, "professional")
                .await
                .unwrap();
        repos
            .users
            .update_password(coach_id, &bcrypt::hash(PASSWORD, 4).unwrap())
            .await
            .unwrap();
        let (_, _, shared) = common::create_test_user_with_plan(
            &res.agent.database,
            "approving-admin@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        repos
            .users
            .update_tenant_id(coach_id, shared)
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("an account in another account's workspace is never the reviewer");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("workspace another account owns")
                && said.contains("--coach-email")
                && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "no group was written"
        );
        assert!(
            repos
                .users
                .manages_roster_admin_grant(coach_id)
                .await
                .unwrap()
                .is_none(),
            "the coach's access was not written"
        );
    }

    /// An adopted coach account that already coaches another group is
    /// refused before anything is written: the reviewers would reach its
    /// athletes through the coach tools (carnet#1511 design call 9).
    #[tokio::test]
    async fn an_adopted_coach_who_coaches_another_group_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, _) = bare_coach(&res).await;
        let (owner, _, owner_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "real-owner@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let agent = create_group_agent(repos, owner, owner_tenant)
            .await
            .unwrap();
        let real = create_coached_group(repos, owner, owner_tenant, &agent, "Real Squad", coach_id)
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a coach of another group is never the reviewer");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("already coaches a group")
                && said.contains("--coach-email")
                && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        let coached = repos.groups.list_groups_coached_by(coach_id).await.unwrap();
        assert_eq!(
            coached.iter().map(|g| g.id).collect::<Vec<_>>(),
            vec![real],
            "no reviewer group was written"
        );
        assert!(
            repos
                .users
                .manages_roster_admin_grant(coach_id)
                .await
                .unwrap()
                .is_none(),
            "the coach's access was not written"
        );
    }

    /// A reviewer whose group holds an athlete the seeder did not make, as an
    /// invite accepted after a run leaves it, is refused on the rerun: the
    /// directory reviewers must never reach a real athlete.
    #[tokio::test]
    async fn a_reviewer_coaching_an_athlete_the_seeder_did_not_make_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (_, group) = seeded_group(&res).await;
        let (stranger, _, stranger_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "real-athlete@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        add_group_member(repos, group, stranger, stranger_tenant)
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("a coach of an athlete the seeder did not make is refused");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("would reach an athlete")
                && said.contains("--coach-email")
                && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
    }

    /// The reviewer's group, its coach taken off, which a rerun would give
    /// its coach back, holds a real athlete: once the run is done the coach
    /// would reach them through the group's live members, so the run is
    /// refused before it writes (carnet#1511 design call 14). The coach
    /// coaches no one and the group stays coachless.
    #[tokio::test]
    async fn a_coachless_reviewer_group_with_a_real_member_is_refused_left_coachless() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant, group) =
            coachless_reviewer_group(&res, store(&res).as_ref()).await;
        let (stranger, _, stranger_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "real-member@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        add_group_member(repos, group, stranger, stranger_tenant)
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a group holding a real athlete is never given its coach back");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("would reach an athlete")
                && said.contains("--coach-email")
                && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        assert!(
            repos
                .groups
                .list_athletes_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "the coach coaches no one"
        );
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "the group was not given its coach back"
        );
        assert_eq!(
            repos
                .groups
                .list_coachless_groups_owned_by(coach_id, coach_tenant)
                .await
                .unwrap()
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            vec![group],
            "the group is left as it was"
        );
    }

    /// An adopted coach account that is a member of another group reads the
    /// peers who share with that room: it is refused before anything is
    /// written, as a coach of another group is.
    #[tokio::test]
    async fn an_adopted_coach_who_belongs_to_another_group_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (owner, _, owner_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "club-owner@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let agent = create_group_agent(repos, owner, owner_tenant)
            .await
            .unwrap();
        let club = create_coachless_group(repos, owner, owner_tenant, &agent, "Real Club")
            .await
            .unwrap();
        add_group_member(repos, club, coach_id, coach_tenant)
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a member of another group is never the reviewer");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("already belongs to a group")
                && said.contains("--coach-email")
                && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "no reviewer group was written"
        );
    }

    /// The real athlete's group, coachless and owned by someone else, with
    /// the athlete in it: the group the paths below would reach.
    async fn real_club(res: &Arc<ServerContext>) -> (Uuid, TenantId) {
        let repos = &res.common.repos;
        let (owner, _, owner_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "club-owner@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let agent = create_group_agent(repos, owner, owner_tenant)
            .await
            .unwrap();
        let club = create_coachless_group(repos, owner, owner_tenant, &agent, "Real Club")
            .await
            .unwrap();
        (club, owner_tenant)
    }

    /// An active invite to `group`, of `kind`, made by `creator`: its id.
    async fn open_invite(
        res: &Arc<ServerContext>,
        group: Uuid,
        tenant: TenantId,
        creator: Uuid,
        kind: GroupInviteKind,
    ) -> Uuid {
        let invite = GroupInvite {
            id: Uuid::new_v4(),
            group_id: group,
            tenant_id: tenant.to_string(),
            code: Uuid::new_v4().simple().to_string()[..8].to_owned(),
            kind,
            created_by: creator,
            expires_at: None,
            max_uses: None,
            use_count: 0,
            is_active: true,
            approves_signup: false,
            created_at: Timestamp::now(),
        };
        res.common
            .repos
            .groups
            .create_invite(&invite)
            .await
            .unwrap();
        invite.id
    }

    /// An invite to `group`, of `kind`, held for `holder` until they join.
    async fn hold_invite(
        res: &Arc<ServerContext>,
        group: Uuid,
        tenant: TenantId,
        holder: Uuid,
        kind: GroupInviteKind,
    ) {
        let invite = open_invite(res, group, tenant, holder, kind).await;
        res.common
            .repos
            .signup_codes
            .hold_group_invite(holder, invite, Timestamp::now())
            .await
            .unwrap();
    }

    /// The run was refused for `reason`, the email masked and the remedy
    /// named, and wrote neither the coach's access nor a group.
    async fn assert_refused_unmodified(
        res: &Arc<ServerContext>,
        coach_id: Uuid,
        refused: &AppError,
        reason: &str,
    ) {
        let repos = &res.common.repos;
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains(reason) && said.contains("--coach-email") && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        assert!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .is_empty(),
            "no reviewer group was written"
        );
        assert!(
            repos
                .users
                .manages_roster_admin_grant(coach_id)
                .await
                .unwrap()
                .is_none(),
            "the coach's access was not written"
        );
    }

    /// An adopted coach account holding a member invite to a real group is
    /// refused before anything is written: the app joins it on the account's
    /// next sign-in, and the reviewers would read the peers who share with
    /// that room (carnet#1511 design call 14). The hold is left as it was.
    #[tokio::test]
    async fn a_held_member_invite_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let (coach_id, _) = bare_coach(&res).await;
        let (club, club_tenant) = real_club(&res).await;
        hold_invite(&res, club, club_tenant, coach_id, GroupInviteKind::Member).await;

        let refused = run(args(PASSWORD), &res.common.repos, store(&res).as_ref())
            .await
            .expect_err("a held invite is never carried into the reviewer");
        assert_refused_unmodified(&res, coach_id, &refused, "holds an invite to a group").await;
        assert!(
            res.common
                .repos
                .signup_codes
                .held_group_invite(coach_id)
                .await
                .unwrap()
                .is_some(),
            "the hold is left as it was"
        );
    }

    /// A held invite to a group's coach seat is refused the same way: the
    /// coach access the run grants is what makes the account eligible for
    /// the seat, which would reach the group's members.
    #[tokio::test]
    async fn a_held_coach_invite_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let (coach_id, _) = bare_coach(&res).await;
        let (club, club_tenant) = real_club(&res).await;
        hold_invite(&res, club, club_tenant, coach_id, GroupInviteKind::Coach).await;

        let refused = run(args(PASSWORD), &res.common.repos, store(&res).as_ref())
            .await
            .expect_err("a held coach invite is never carried into the reviewer");
        assert_refused_unmodified(&res, coach_id, &refused, "holds an invite to a group").await;
    }

    /// A coachless group the account owns under any other name is refused
    /// before anything is written: the coach access the run grants lets its
    /// owner claim the group's coach seat, and whoever is or joins in it
    /// would be reached. The group is left coachless.
    #[tokio::test]
    async fn an_owned_coachless_group_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let agent = create_group_agent(repos, coach_id, coach_tenant)
            .await
            .unwrap();
        let other = create_coachless_group(repos, coach_id, coach_tenant, &agent, "Real Club")
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("an owned coachless group is never carried into the reviewer");
        assert_refused_unmodified(&res, coach_id, &refused, "owns a coachless group").await;
        assert_eq!(
            repos
                .groups
                .list_coachless_groups_owned_by(coach_id, coach_tenant)
                .await
                .unwrap()
                .iter()
                .map(|group| group.id)
                .collect::<Vec<_>>(),
            vec![other],
            "the owned group is left as it was"
        );
    }

    /// An archive keeps a group's coach and its live members, and switching
    /// it back on hands them back, so the reach preflight counts an archived
    /// group the adopted account coaches, owns or belongs to (round-12
    /// seeder finding): each is refused before anything is written, the
    /// group left as it was.
    #[tokio::test]
    async fn an_archived_group_the_account_holds_is_refused_unmodified() {
        for held in ["coaches", "owns", "belongs to"] {
            let res = create_test_server_resources().await.unwrap();
            let repos = &res.common.repos;
            let (coach_id, coach_tenant) = bare_coach(&res).await;
            let (athlete, _, athlete_tenant) = common::create_test_user_with_plan(
                &res.agent.database,
                "real-athlete@directory-seed.test",
                "professional",
            )
            .await
            .unwrap();
            let club = if held == "owns" {
                let agent = create_group_agent(repos, coach_id, coach_tenant)
                    .await
                    .unwrap();
                create_coachless_group(repos, coach_id, coach_tenant, &agent, "Real Club")
                    .await
                    .unwrap()
            } else {
                let (club, owner_tenant) = real_club(&res).await;
                if held == "coaches" {
                    assert!(
                        repos
                            .groups
                            .set_group_coach_user(&club.to_string(), Some(coach_id), owner_tenant)
                            .await
                            .unwrap(),
                        "the coach takes the club's coach seat"
                    );
                } else {
                    add_group_member(repos, club, coach_id, coach_tenant)
                        .await
                        .unwrap();
                }
                club
            };
            add_group_member(repos, club, athlete, athlete_tenant)
                .await
                .unwrap();
            execute(
                &res,
                "UPDATE coaching_groups SET is_active = FALSE WHERE id = $1",
                &[Bound::Id(club)],
            )
            .await;

            let refused = run(args(PASSWORD), repos, store(&res).as_ref())
                .await
                .expect_err(
                    "an archived group the account holds is reach a reactivation hands back",
                );
            assert_refused_unmodified(
                &res,
                coach_id,
                &refused,
                "holds another group, archived or owned",
            )
            .await;
            let members = repos.groups.list_members(&club.to_string()).await.unwrap();
            assert!(
                members
                    .iter()
                    .any(|m| m.user_id == athlete && m.left_at.is_none()),
                "{held}: the archived group is left as it was"
            );
        }
    }

    /// An account added to another account's private thread reads that
    /// account's rows there: it is refused before anything is written.
    #[tokio::test]
    async fn a_participant_in_anothers_thread_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, _) = bare_coach(&res).await;
        let (owner, _, owner_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "thread-owner@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let thread = repos
            .chat
            .create_conversation(
                &owner.to_string(),
                owner_tenant,
                "their thread",
                "model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        repos
            .chat
            .add_participant(
                &thread,
                owner_tenant,
                &coach_id.to_string(),
                &owner.to_string(),
            )
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a participant in another's thread is never the reviewer");
        assert_refused_unmodified(
            &res,
            coach_id,
            &refused,
            "shares a private thread with an account",
        )
        .await;
    }

    /// An account that owns a private thread another account was added to
    /// shares it with them: that account keeps reading and posting beside
    /// the reviewers, so the run is refused before anything is written.
    #[tokio::test]
    async fn an_owner_of_a_thread_another_account_joined_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (guest, _, _) = common::create_test_user_with_plan(
            &res.agent.database,
            "thread-guest@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        repos
            .users
            .update_tenant_id(guest, coach_tenant)
            .await
            .unwrap();
        let thread = repos
            .chat
            .create_conversation(
                &coach_id.to_string(),
                coach_tenant,
                "the coach's thread",
                "model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        repos
            .chat
            .add_participant(
                &thread,
                coach_tenant,
                &guest.to_string(),
                &coach_id.to_string(),
            )
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a thread the coach shares with another account is refused");
        assert_refused_unmodified(
            &res,
            coach_id,
            &refused,
            "shares a private thread with an account",
        )
        .await;
    }

    /// Make `user` a plain member of `tenant` (a `tenant_users` row), as a
    /// workspace invitation does, leaving their own workspace as it is.
    async fn add_tenant_membership(res: &Arc<ServerContext>, tenant: TenantId, user: Uuid) {
        const INSERT: &str = "INSERT INTO tenant_users \
                              (id, tenant_id, user_id, role, invited_at, joined_at) \
                              VALUES ($1, $2, $3, 'member', $4, $5)";
        let now = Timestamp::now();
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(INSERT)
                    .bind(Uuid::new_v4().to_string())
                    .bind(tenant.to_string())
                    .bind(user.to_string())
                    .bind(now.to_rfc3339())
                    .bind(now.to_rfc3339())
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(INSERT)
                    .bind(Uuid::new_v4())
                    .bind(Uuid::parse_str(&tenant.to_string()).unwrap())
                    .bind(user)
                    .bind(now)
                    .bind(now)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    /// A coachless group the account owns in another workspace it is a
    /// member of is refused like one in its home workspace: the coach access
    /// the run grants lets the account claim its coach seat once it acts in
    /// that workspace. The group is left coachless.
    #[tokio::test]
    async fn an_owned_coachless_group_in_another_workspace_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, _) = bare_coach(&res).await;
        let (_, club_tenant) = real_club(&res).await;
        add_tenant_membership(&res, club_tenant, coach_id).await;
        let agent = create_group_agent(repos, coach_id, club_tenant)
            .await
            .unwrap();
        let other = create_coachless_group(repos, coach_id, club_tenant, &agent, "Away Club")
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("an owned coachless group in any workspace is refused");
        assert_refused_unmodified(&res, coach_id, &refused, "owns a coachless group").await;
        assert_eq!(
            repos
                .groups
                .get_group(&other.to_string(), club_tenant)
                .await
                .unwrap()
                .map(|group| group.coach_user_id),
            Some(None),
            "the owned group is left coachless"
        );
    }

    /// The reviewer's group after a complete run through `live`, its coach
    /// taken off as a run that failed before attaching them, or an admin,
    /// leaves it: the group a rerun gives its coach back. The coach's id and
    /// tenant, and the group's id.
    async fn coachless_reviewer_group(
        res: &Arc<ServerContext>,
        live: &dyn LiveActivityStore,
    ) -> (Uuid, TenantId, Uuid) {
        let repos = &res.common.repos;
        run(args(PASSWORD), repos, live).await.unwrap();
        let (coach_id, group) = seeded_group(res).await;
        let (_, tenant) = seeded(res, COACH).await;
        assert!(
            repos
                .groups
                .set_group_coach_user(&group.to_string(), None, tenant)
                .await
                .unwrap(),
            "the fixture takes the coach off the group"
        );
        (coach_id, tenant, group)
    }

    /// A coachless active group `coach_id` owns in `tenant`: its id.
    async fn owned_coachless_group(
        res: &Arc<ServerContext>,
        coach_id: Uuid,
        tenant: TenantId,
    ) -> Uuid {
        let repos = &res.common.repos;
        let agent = create_group_agent(repos, coach_id, tenant).await.unwrap();
        create_coachless_group(repos, coach_id, tenant, &agent, "A coachless squad")
            .await
            .unwrap()
    }

    /// An open invite into the reviewer's group is refused before the rerun
    /// writes, whoever made it and of either kind: whoever redeems it later
    /// joins the group the reviewers read. The group is left coachless and
    /// the invite open.
    #[tokio::test]
    async fn an_open_invite_into_the_reviewers_group_is_refused_unmodified() {
        for kind in [GroupInviteKind::Member, GroupInviteKind::Coach] {
            let res = create_test_server_resources().await.unwrap();
            let repos = &res.common.repos;
            let (coach_id, coach_tenant, group) =
                coachless_reviewer_group(&res, store(&res).as_ref()).await;
            open_invite(&res, group, coach_tenant, coach_id, kind).await;

            let refused = run(args(PASSWORD), repos, store(&res).as_ref())
                .await
                .expect_err("an open invite into the reviewer's group is refused");
            let said = format!("{refused}");
            assert!(
                said.contains("has an invite someone could still join it through")
                    && said.contains("--coach-email")
                    && !said.contains(COACH),
                "refused with the reason and the remedy, the email masked: {said}"
            );
            assert_eq!(
                repos
                    .groups
                    .get_group(&group.to_string(), coach_tenant)
                    .await
                    .unwrap()
                    .map(|group| group.coach_user_id),
                Some(None),
                "the group is left coachless"
            );
            assert!(
                repos
                    .groups
                    .list_invites(&group.to_string())
                    .await
                    .unwrap()
                    .iter()
                    .all(|invite| invite.is_active),
                "the invite is left open"
            );
        }
    }

    /// A hold another account took on an invite into the reviewer's group
    /// is refused even once the invite was retired: the hold is redeemed on
    /// that account's next sign-in, so its existence is the path.
    #[tokio::test]
    async fn a_hold_on_a_retired_invite_into_the_reviewers_group_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant, group) =
            coachless_reviewer_group(&res, store(&res).as_ref()).await;
        let (holder, _, _) = common::create_test_user_with_plan(
            &res.agent.database,
            "invite-holder@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let invite =
            open_invite(&res, group, coach_tenant, coach_id, GroupInviteKind::Member).await;
        repos
            .signup_codes
            .hold_group_invite(holder, invite, Timestamp::now())
            .await
            .unwrap();
        assert!(
            repos
                .groups
                .deactivate_invite(&group.to_string(), &invite.to_string())
                .await
                .unwrap(),
            "the held invite is deactivated"
        );

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a hold on the reviewer's group's invite is refused");
        assert!(
            format!("{refused}").contains("has an invite someone could still join it through"),
            "refused for the hold: {refused}"
        );
    }

    /// A retired invite into the reviewer's group that nobody holds opens
    /// no path: the rerun gives the group its coach back and completes it.
    #[tokio::test]
    async fn a_retired_invite_nobody_holds_does_not_refuse_the_rerun() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        let (coach_id, coach_tenant, group) = coachless_reviewer_group(&res, live.as_ref()).await;
        let invite =
            open_invite(&res, group, coach_tenant, coach_id, GroupInviteKind::Member).await;
        assert!(
            repos
                .groups
                .deactivate_invite(&group.to_string(), &invite.to_string())
                .await
                .unwrap(),
            "the open invite is deactivated"
        );

        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        assert_fully_seeded(&res, live.as_ref()).await;
        assert_eq!(
            repos.groups.list_groups_coached_by(coach_id).await.unwrap()[0].id,
            group,
            "the reviewer's group is the one completed"
        );
    }

    /// Giving a coachless group its coach back is one write conditional on
    /// the group still being active with no coach: a coach a racing writer
    /// attached first is kept, an archived group is left alone, and the
    /// write says it matched nothing, which the seeder refuses on.
    #[tokio::test]
    async fn the_coach_is_given_back_only_to_a_group_still_coachless() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (rival, _, _) = common::create_test_user_with_plan(
            &res.agent.database,
            "rival-coach@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let group = owned_coachless_group(&res, coach_id, coach_tenant).await;
        let id = group.to_string();
        assert!(
            repos
                .groups
                .set_group_coach_user(&id, Some(rival), coach_tenant)
                .await
                .unwrap(),
            "the rival takes the group's coach seat"
        );
        assert!(
            !repos
                .groups
                .attach_coach_to_coachless_group(&id, coach_id, coach_tenant)
                .await
                .unwrap(),
            "a coach attached first is never overwritten"
        );
        assert_eq!(
            repos
                .groups
                .get_group(&id, coach_tenant)
                .await
                .unwrap()
                .and_then(|group| group.coach_user_id),
            Some(rival),
            "the racing writer's coach is kept"
        );

        let archived = owned_coachless_group(&res, coach_id, coach_tenant)
            .await
            .to_string();
        assert!(
            repos
                .groups
                .delete_group(&archived, coach_tenant)
                .await
                .unwrap(),
            "the archived group is deleted"
        );
        assert!(
            !repos
                .groups
                .attach_coach_to_coachless_group(&archived, coach_id, coach_tenant)
                .await
                .unwrap(),
            "an archived group is never given a coach"
        );

        let coachless = owned_coachless_group(&res, coach_id, coach_tenant)
            .await
            .to_string();
        assert!(
            repos
                .groups
                .attach_coach_to_coachless_group(&coachless, coach_id, coach_tenant)
                .await
                .unwrap(),
            "a group still coachless is given its coach"
        );
    }

    /// A fresh account passes every reach check and ends with exactly the
    /// three seeded members; a complete rerun passes too and reaches no one
    /// else.
    #[tokio::test]
    async fn a_fresh_account_and_a_rerun_end_with_exactly_the_three() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (coach_id, _) = seeded(&res, COACH).await;
        let reached: BTreeSet<Uuid> = repos
            .groups
            .list_athletes_coached_by(coach_id)
            .await
            .unwrap()
            .into_iter()
            .collect();
        let mut seeded_members = BTreeSet::new();
        for (number, ..) in MEMBERS {
            seeded_members.insert(seeded(&res, &member_email(&res, number).await).await.0);
        }
        assert_eq!(
            reached, seeded_members,
            "the coach reaches the three seeded members only"
        );
    }

    /// An account that coached a real group and was detached from it keeps
    /// its thread about that group's athlete: the thread lists to it titled
    /// with the athlete's name and holds its notes about them, so the run is
    /// refused before anything is written (round-12 seeder finding).
    #[tokio::test]
    async fn a_detached_coachs_thread_about_a_real_athlete_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (club, _) = real_club(&res).await;
        let (athlete, _, athlete_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "former-athlete@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        add_group_member(repos, club, athlete, athlete_tenant)
            .await
            .unwrap();
        let thread = repos
            .chat
            .create_conversation(
                &coach_id.to_string(),
                coach_tenant,
                "Former Athlete",
                "model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        execute(
            &res,
            "UPDATE chat_conversations SET subject_user_id = $1, subject_group_id = $2 \
             WHERE id = $3",
            &[Bound::Id(athlete), Bound::Id(club), Bound::Text(thread)],
        )
        .await;

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a thread about a real athlete is reach the reviewer would keep");
        assert_refused_unmodified(&res, coach_id, &refused, "holds a thread about an athlete")
            .await;
    }

    /// The reviewer's group, bound to a messaging chat it was made for (no
    /// web connection), enrols every linked sender who writes there with
    /// coach sharing on: the rerun is refused before it writes, the group
    /// left coachless (round-12 seeder finding).
    #[tokio::test]
    async fn a_reviewer_group_a_messaging_chat_enrols_into_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (_, coach_tenant, group) = coachless_reviewer_group(&res, store(&res).as_ref()).await;
        execute(
            &res,
            "UPDATE coaching_groups SET channel_type = 'telegram', \
             channel_chat_id = '-100777' WHERE id = $1",
            &[Bound::Id(group)],
        )
        .await;

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a chat that enrols whoever writes is reach the reviewer would keep");
        let said = format!("{refused}");
        assert!(
            said.contains("bound to a messaging chat that enrols whoever writes in it")
                && said.contains("--coach-email")
                && !said.contains(COACH),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        assert_eq!(
            repos
                .groups
                .get_group(&group.to_string(), coach_tenant)
                .await
                .unwrap()
                .map(|group| group.coach_user_id),
            Some(None),
            "the group is left coachless"
        );
    }

    /// Round-13 seeder major (call 21): an account that left a real group,
    /// or stopped coaching it, keeps its room conversation there, listed to
    /// it with the other members' words said to the room. It is refused
    /// before anything is written.
    #[tokio::test]
    async fn an_account_keeping_another_groups_room_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (owner, _, owner_tenant) = common::create_test_user_with_plan(
            &res.agent.database,
            "room-owner@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let agent = create_group_agent(repos, owner, owner_tenant)
            .await
            .unwrap();
        let left = create_coachless_group(repos, owner, owner_tenant, &agent, "A real squad")
            .await
            .unwrap();
        repos
            .chat
            .create_conversation(
                &coach_id.to_string(),
                coach_tenant,
                "the squad's room",
                "model",
                None,
                Some(&left.to_string()),
            )
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("a room the account keeps in another group is refused");
        assert_refused_unmodified(
            &res,
            coach_id,
            &refused,
            "keeps a room conversation of another group",
        )
        .await;
    }

    /// Round-13 seeder major (call 21), narrowed by call 29: a coaching
    /// platform's roster read through the account's own connection names
    /// every athlete on it, linked or not, so an account holding a
    /// coach-roster provider's connection is refused before anything is
    /// written.
    #[tokio::test]
    async fn an_account_holding_a_provider_connection_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken::new(
                coach_id,
                coach_tenant.to_string(),
                INTERVALS_ICU.to_owned(),
                "coach-platform-access".to_owned(),
                None,
                None,
                None,
            ))
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("an account with a provider connection is refused");
        assert_refused_unmodified(
            &res,
            coach_id,
            &refused,
            "holds a coaching-platform connection",
        )
        .await;
    }

    /// carnet#1511 call 29: only a coach-roster provider's connection lists
    /// athletes the account did not make. A Strava connection reads the
    /// account's own training alone, so the run seeds the reviewer.
    #[tokio::test]
    async fn an_account_holding_a_non_roster_connection_is_seeded() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        repos
            .oauth_tokens
            .upsert_token(&UserOAuthToken::new(
                coach_id,
                coach_tenant.to_string(),
                STRAVA.to_owned(),
                "own-training-access".to_owned(),
                None,
                None,
                None,
            ))
            .await
            .unwrap();

        run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect("bug: a Strava connection, which lists no roster, refused the reviewer");
        assert_eq!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .len(),
            1,
            "the reviewer's group was seeded"
        );
    }

    /// carnet#1511 call 29: an adopted account whose own conversation
    /// carries a read of an athlete the seeder did not make keeps serving
    /// that athlete's data to whoever signs in as it, so the run is refused
    /// before anything is written. An ordinary thread with nobody else in
    /// it, not a subject thread and not a room.
    #[tokio::test]
    async fn an_own_conversation_carrying_a_real_athletes_read_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (club, _) = real_club(&res).await;
        let (athlete, _, _) = common::create_test_user_with_plan(
            &res.agent.database,
            "real-athlete@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let thread = repos
            .chat
            .create_conversation(
                &coach_id.to_string(),
                coach_tenant,
                "How is the squad doing?",
                "model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        let coach = coach_id.to_string();
        repos
            .chat
            .add_message_about(
                &AddMessageParams {
                    tenant_id: coach_tenant,
                    conversation_id: &thread,
                    user_id: &coach,
                    role: "assistant",
                    content: "The athlete rode 3h at 250W and their HRV is down",
                    token_count: None,
                    finish_reason: Some("stop"),
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    channel_type: None,
                    about_user_id: None,
                    turn_attempted_write: None,
                    ai_consent_sources: &[],
                },
                &[CarriedRead::coach(athlete, club, coach_id)],
            )
            .await
            .unwrap();

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("bug: a conversation holding a real athlete's data passed the preflight");
        assert_refused_unmodified(
            &res,
            coach_id,
            &refused,
            "keeps a conversation about an athlete",
        )
        .await;
    }

    /// carnet#1511 call 29: an athlete added to the account's own thread,
    /// who wrote there and was then removed, left their own words in it.
    /// Those rows are authored by them and still served to the owner, so the
    /// run is refused like any other carried read.
    #[tokio::test]
    async fn an_own_thread_keeping_a_removed_athletes_words_is_refused_unmodified() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let (coach_id, coach_tenant) = bare_coach(&res).await;
        let (athlete, _, _) = common::create_test_user_with_plan(
            &res.agent.database,
            "removed-athlete@directory-seed.test",
            "professional",
        )
        .await
        .unwrap();
        let coach = coach_id.to_string();
        let athlete_id = athlete.to_string();
        let thread = repos
            .chat
            .create_conversation(&coach, coach_tenant, "Check-in", "model", None, None)
            .await
            .unwrap()
            .id;
        repos
            .chat
            .add_participant(&thread, coach_tenant, &athlete_id, &coach)
            .await
            .unwrap();
        repos
            .chat
            .add_message_about(
                &AddMessageParams {
                    tenant_id: coach_tenant,
                    conversation_id: &thread,
                    user_id: &athlete_id,
                    role: "user",
                    content: "My resting HR has been 62 since the knee injury",
                    token_count: None,
                    finish_reason: None,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    channel_type: None,
                    about_user_id: None,
                    turn_attempted_write: None,
                    ai_consent_sources: &[],
                },
                &[],
            )
            .await
            .unwrap();
        assert!(
            repos
                .chat
                .remove_participant(&thread, coach_tenant, &athlete_id)
                .await
                .unwrap(),
            "the athlete is removed from the thread"
        );

        let refused = run(args(PASSWORD), repos, store(&res).as_ref())
            .await
            .expect_err("bug: a removed athlete's own words passed the preflight");
        assert_refused_unmodified(
            &res,
            coach_id,
            &refused,
            "keeps a conversation about an athlete",
        )
        .await;
    }

    /// The seeder's sessions of `member` the cache serves, newest first.
    async fn seeded_sessions(
        res: &Arc<ServerContext>,
        member: Uuid,
        tenant: TenantId,
    ) -> Vec<Activity> {
        res.common
            .repos
            .activity_cache
            .get_cached_activities(
                member,
                &tenant,
                Some(INTERVALS_ICU),
                Timestamp::now() - Elapsed::days(400),
                Timestamp::now() + Elapsed::days(1),
                100,
            )
            .await
            .unwrap()
            .into_iter()
            .filter(|session| session.id().starts_with("directory-reviewer-"))
            .collect()
    }

    /// carnet#1511 call 29: the seeded training is keyed by calendar date
    /// over a rolling window, so a rerun after the sessions went stale
    /// replaces those that fell out of the window and the roster reads
    /// recent training again. Ava's sessions are put back where a run
    /// weeks ago left them.
    #[tokio::test]
    async fn a_rerun_after_the_sessions_went_stale_brings_them_into_the_window() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (ava, ava_tenant) = seeded(&res, &member_email(&res, 1).await).await;
        for session in seeded_sessions(&res, ava, ava_tenant).await {
            assert!(
                live.discard(ava, ava_tenant, INTERVALS_ICU, session.id())
                    .await
                    .unwrap(),
                "the seeded session is discarded"
            );
        }
        // What a run 40 days ago wrote: sessions keyed by their calendar
        // date, all older than the window a rerun keeps.
        let stale: Vec<Activity> = [1_i64, 4, 7, 10, 13, 16, 19]
            .iter()
            .map(|days_ago| {
                let start = Timestamp::now() - Elapsed::days(40 + days_ago);
                let day = start.epoch_millis().div_euclid(86_400_000);
                ActivityBuilder::new(
                    format!("directory-reviewer-{}-day{day}", ava.simple()),
                    "Endurance ride".to_owned(),
                    SportType::Ride,
                    start,
                    3_600,
                    INTERVALS_ICU.to_owned(),
                )
                .distance_meters(30_000.0)
                .build()
            })
            .collect();
        live.ingest(
            ava,
            ava_tenant,
            INTERVALS_ICU,
            &stale,
            Timestamp::now(),
            WriteOrigin::Import,
        )
        .await
        .unwrap();

        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let sessions = seeded_sessions(&res, ava, ava_tenant).await;
        let week_ago = Timestamp::now() - Elapsed::days(7);
        assert!(
            sessions
                .iter()
                .any(|session| session.start_date() >= week_ago),
            "bug: a rerun left Ava's seeded training stale: {:?}",
            sessions
                .iter()
                .map(|s| s.id().to_owned())
                .collect::<Vec<_>>()
        );
        let oldest = Timestamp::now() - Elapsed::days(22);
        assert!(
            sessions
                .iter()
                .all(|session| session.start_date() >= oldest),
            "bug: sessions out of the window outlived the rerun: {:?}",
            sessions
                .iter()
                .map(|s| s.id().to_owned())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            live.held(ava, ava_tenant, INTERVALS_ICU).await.unwrap(),
            SESSIONS,
            "the window holds the seeder's sessions once"
        );
    }

    /// Assert `refused` names the workspace of `email` as inactive or
    /// ownerless, with the remedy, the email masked, and no bare not-found.
    fn assert_workspace_refusal(refused: &AppError, email: &str) {
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("inactive or has no owner")
                && said.contains("--coach-email")
                && !said.contains(email)
                && !said.contains("ResourceNotFound"),
            "refused with the reason and the remedy, the email masked, never a bare not-found: \
             {said}"
        );
    }

    /// The coach's own workspace switched off, or left with no owner,
    /// cannot hold the reviewer's group: a rerun refuses with the reason and
    /// the remedy instead of a bare not-found, and writes nothing. The same
    /// holds of a member's workspace.
    #[tokio::test]
    async fn an_inactive_or_ownerless_workspace_is_refused_with_its_remedy() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let (coach_id, coach_tenant) = seeded(&res, COACH).await;
        let groups_before = repos.groups.list_groups_coached_by(coach_id).await.unwrap();

        execute(
            &res,
            "UPDATE tenants SET is_active = FALSE WHERE id = $1",
            &[Bound::Id(coach_tenant.as_uuid())],
        )
        .await;
        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("an inactive workspace is never the reviewer's");
        assert_workspace_refusal(&refused, COACH);
        execute(
            &res,
            "UPDATE tenants SET is_active = TRUE WHERE id = $1",
            &[Bound::Id(coach_tenant.as_uuid())],
        )
        .await;

        execute(
            &res,
            "UPDATE tenant_users SET role = 'member' WHERE tenant_id = $1 AND role = 'owner'",
            &[Bound::Id(coach_tenant.as_uuid())],
        )
        .await;
        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("a workspace with no owner is never the reviewer's");
        assert_workspace_refusal(&refused, COACH);
        execute(
            &res,
            "UPDATE tenant_users SET role = 'owner' WHERE tenant_id = $1 AND user_id = $2",
            &[Bound::Id(coach_tenant.as_uuid()), Bound::Id(coach_id)],
        )
        .await;

        let theirs = member_email(&res, 1).await;
        let (_, member_tenant) = seeded(&res, &theirs).await;
        execute(
            &res,
            "UPDATE tenants SET is_active = FALSE WHERE id = $1",
            &[Bound::Id(member_tenant.as_uuid())],
        )
        .await;
        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("a member in an inactive workspace is refused");
        assert_workspace_refusal(&refused, &theirs);
        assert_eq!(
            repos
                .groups
                .list_groups_coached_by(coach_id)
                .await
                .unwrap()
                .len(),
            groups_before.len(),
            "no group was written"
        );
    }

    /// An email a deleted account left a live tombstone on is never given a
    /// new active account by the seeder: sign-up holds such an address for
    /// review, and the seeder would hand it straight back. The account at
    /// the coach email signed in with the reviewer's password, then was
    /// deleted. The rerun refuses
    /// with the reason and the remedy, the email masked, and creates nothing.
    #[tokio::test]
    async fn a_deleted_accounts_email_is_never_given_a_new_account() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        let (coach_id, _) = bare_coach(&res).await;
        delete_account(repos, coach_id, None).await.unwrap();
        assert!(
            repos
                .import_tombstones
                .find_live(TombstoneKind::Email, None, &email_identity_hash(COACH))
                .await
                .unwrap()
                .is_some(),
            "premise: the deletion left a live tombstone on the coach's email"
        );
        for _ in 0..2 {
            let refused = run(args(PASSWORD), repos, live.as_ref())
                .await
                .expect_err("a tombstoned email is never given a new account");
            let said = format!("{refused} {refused:?}");
            assert!(
                said.contains("deleted account")
                    && said.contains("clear-tombstone")
                    && said.contains("--coach-email")
                    && !said.contains(COACH),
                "refused with the reason and the remedy, the email masked: {said}"
            );
            assert!(
                repos
                    .seeder
                    .seed_check_user_exists(COACH)
                    .await
                    .unwrap()
                    .is_none(),
                "bug: an active account was created at a deleted account's email"
            );
        }
    }

    /// A member's email that a deleted account left a live tombstone on,
    /// its slug free again, is never given a new account: the rerun refuses
    /// in the preflight before writing anything.
    #[tokio::test]
    async fn a_deleted_members_email_is_never_given_a_new_account() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        let live = store(&res);
        run(args(PASSWORD), repos, live.as_ref()).await.unwrap();
        let theirs = member_email(&res, 2).await;
        let (member, member_tenant) = seeded(&res, &theirs).await;
        delete_account(repos, member, None).await.unwrap();
        execute(
            &res,
            "DELETE FROM tenant_users WHERE tenant_id = $1",
            &[Bound::Id(member_tenant.as_uuid())],
        )
        .await;
        execute(
            &res,
            "DELETE FROM tenants WHERE id = $1",
            &[Bound::Id(member_tenant.as_uuid())],
        )
        .await;
        assert!(
            repos
                .import_tombstones
                .find_live(TombstoneKind::Email, None, &email_identity_hash(&theirs))
                .await
                .unwrap()
                .is_some(),
            "premise: the deletion left a live tombstone on the member's email"
        );
        let refused = run(args(PASSWORD), repos, live.as_ref())
            .await
            .expect_err("a tombstoned member email is never given a new account");
        let said = format!("{refused} {refused:?}");
        assert!(
            said.contains("deleted account")
                && said.contains("--coach-email")
                && !said.contains(&theirs),
            "refused with the reason and the remedy, the email masked: {said}"
        );
        assert!(
            repos
                .seeder
                .seed_check_user_exists(&theirs)
                .await
                .unwrap()
                .is_none(),
            "bug: an active account was created at a deleted member's email"
        );
    }
}
