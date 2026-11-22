// ABOUTME: Over /api/chat an outside client acting for a coach is never served a thread about a member who does not share
// ABOUTME: Such a thread is left out of the list and answered as one that does not exist; the coach's own app is unchanged
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! EFVP 2026-11-17, decision 9.4c, over the chat REST routes. A coach's
//! thread about one athlete is titled with the athlete's name and is about
//! them throughout. An API key the coach holds reads those routes, so it
//! listed the thread about a member who had stopped sharing, returned their
//! name with it, and refused the plan card naming them ("… has stopped
//! sharing"). For an outside client such a thread is now left out of the
//! list and its count, and every route that names it answers as for a
//! conversation that does not exist; a route keyed by the athlete answers as
//! for an athlete who is not a member. The coach's own session in Dravr's
//! app still lists, opens and is told about every thread.

#[cfg(feature = "tools-groups")]
mod coach_outside_client_chat_tests {
    use crate::common::{self, create_test_server_resources};
    use crate::helpers::axum_test::AxumTestRequest;
    use crate::helpers::coach_roster_fixtures::{
        self as roster, coach, coached_group, group_agent, join, person, Person,
    };

    use std::sync::Arc;

    use axum::http::StatusCode;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::chat::ChatRoutes;
    use pierre_mcp_server::routes::coach_replies::CoachReplyRoutes;
    use pierre_routes_groups::GroupRoutes;
    use serde_json::{json, Value};
    use uuid::Uuid;

    /// The member who shares their training with the coach.
    const SHARING_NAME: &str = "Alex Sharing";

    /// The member who stopped sharing: never named to an outside client.
    const QUIET_NAME: &str = "Quinn Quiet";

    struct World {
        res: Arc<ServerContext>,
        coach: Person,
        quiet: Person,
        group: Uuid,
        /// The coach's thread about the member who shares.
        shared_thread: String,
        /// The coach's thread about the member who stopped sharing.
        quiet_thread: String,
        /// An API key the coach holds: the whole header.
        api_key: String,
    }

    impl World {
        async fn new() -> Self {
            let res = create_test_server_resources().await.unwrap();
            let casey = coach(&res, "casey@outside-client-chat.test", "Casey Coach").await;
            let alex = person(&res, "alex@outside-client-chat.test", SHARING_NAME).await;
            let quinn = person(&res, "quinn@outside-client-chat.test", QUIET_NAME).await;
            let agent = group_agent(&res, &casey).await;
            let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
            join(&res, group, alex.id, alex.tenant, true).await;
            join(&res, group, quinn.id, quinn.tenant, true).await;
            let api_key = common::issue_test_api_key(&res.agent.database, casey.id, "coach script")
                .await
                .unwrap();
            let mut world = Self {
                res,
                coach: casey,
                quiet: quinn,
                group,
                shared_thread: String::new(),
                quiet_thread: String::new(),
                api_key,
            };
            world.shared_thread = world.open_thread(alex.id).await;
            world.quiet_thread = world.open_thread(world.quiet.id).await;
            // Quinn stops sharing once the coach has a thread about her.
            assert!(
                world
                    .res
                    .common
                    .repos
                    .groups
                    .update_coach_sharing_consent(&group.to_string(), world.quiet.id, false)
                    .await
                    .unwrap(),
                "fixture: the member's coach sharing is turned off"
            );
            world
        }

        /// The coach's thread about `athlete`, opened from the app.
        async fn open_thread(&self, athlete: Uuid) -> String {
            let (status, body) = self
                .rest(
                    AxumTestRequest::post(&format!("/api/chat/athletes/{athlete}/thread"))
                        .json(&json!({ "group_id": self.group })),
                    &self.coach.auth,
                )
                .await;
            assert_eq!(status, StatusCode::OK, "fixture: the thread opens: {body}");
            body["id"].as_str().unwrap().to_owned()
        }

        /// A chat route called with `authorization`: its status and body.
        async fn rest(&self, request: AxumTestRequest, authorization: &str) -> (StatusCode, Value) {
            let response = request
                .header("authorization", authorization)
                .send(ChatRoutes::routes(Arc::clone(&self.res)))
                .await;
            let status = response.status_code();
            let text = response.body_text();
            let mut body = serde_json::from_str(&text).unwrap_or(Value::String(text));
            // When an error was answered says nothing about what it answers.
            if let Some(error) = body.as_object_mut() {
                error.remove("timestamp");
            }
            (status, body)
        }

        /// A group route read with `authorization`: its status and body.
        async fn group_get(&self, uri: &str, authorization: &str) -> (StatusCode, Value) {
            let response = AxumTestRequest::get(uri)
                .header("authorization", authorization)
                .send(GroupRoutes::routes(Arc::clone(&self.res)))
                .await;
            let status = response.status_code();
            let text = response.body_text();
            (
                status,
                serde_json::from_str(&text).unwrap_or(Value::String(text)),
            )
        }

        async fn get(&self, uri: &str, authorization: &str) -> (StatusCode, Value) {
            self.rest(AxumTestRequest::get(uri), authorization).await
        }

        /// The coach's reply review read with `authorization`.
        async fn replies_get(&self, uri: &str, authorization: &str) -> (StatusCode, Value) {
            let response = AxumTestRequest::get(uri)
                .header("authorization", authorization)
                .send(CoachReplyRoutes::routes(Arc::clone(&self.res)))
                .await;
            let status = response.status_code();
            let text = response.body_text();
            (
                status,
                serde_json::from_str(&text).unwrap_or(Value::String(text)),
            )
        }

        /// Whether `body` names the member who stopped sharing, by name or id.
        fn names_quiet(&self, body: &Value) -> bool {
            let text = body.to_string();
            text.contains(QUIET_NAME) || text.contains(&self.quiet.id.to_string())
        }

        /// The ids of the conversations `authorization` is listed, and the
        /// list's total.
        async fn listed(&self, authorization: &str) -> (Vec<String>, i64, Value) {
            let (status, body) = self
                .get("/api/chat/conversations?limit=50", authorization)
                .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            let ids = body["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|conv| conv["id"].as_str().unwrap().to_owned())
                .collect();
            (ids, body["total"].as_i64().unwrap(), body)
        }
    }

    /// Every way the routes keyed by a conversation can answer about one.
    fn conversation_routes(conversation: &str) -> Vec<String> {
        [
            "",
            "/messages",
            "/topics",
            "/participants",
            "/subject-plan",
            "/subject-plan/families",
            "/verdicts",
        ]
        .iter()
        .map(|suffix| format!("/api/chat/conversations/{conversation}{suffix}"))
        .collect()
    }

    #[tokio::test]
    async fn an_outside_client_is_not_listed_a_thread_about_a_member_who_does_not_share() {
        let w = World::new().await;

        let (ids, total, body) = w.listed(&w.api_key).await;
        assert!(ids.contains(&w.shared_thread), "{body}");
        assert!(
            !ids.contains(&w.quiet_thread),
            "bug: the thread about the member who stopped sharing is listed: {body}"
        );
        assert!(
            !body.to_string().contains(QUIET_NAME),
            "bug: the list names the member who stopped sharing: {body}"
        );
        assert_eq!(
            total,
            i64::try_from(ids.len()).unwrap(),
            "bug: the total counts a thread the client is not listed: {body}"
        );

        // In Dravr's own app the coach is still listed both threads.
        let (ids, total, body) = w.listed(&w.coach.auth).await;
        assert!(
            ids.contains(&w.shared_thread) && ids.contains(&w.quiet_thread),
            "the coach's own app lists every thread: {body}"
        );
        assert_eq!(
            total,
            i64::try_from(ids.len()).unwrap(),
            "bug: the total counts what the coach's own app lists"
        );
    }

    #[tokio::test]
    async fn every_route_answers_an_outside_client_as_if_the_thread_did_not_exist() {
        let w = World::new().await;
        let nowhere = Uuid::new_v4().to_string();

        let mut named = Vec::new();
        for (quiet, absent) in conversation_routes(&w.quiet_thread)
            .into_iter()
            .zip(conversation_routes(&nowhere))
        {
            let (status, body) = w.get(&quiet, &w.api_key).await;
            let (absent_status, absent_body) = w.get(&absent, &w.api_key).await;
            if status != absent_status
                || body.to_string().replace(&w.quiet_thread, "<id>")
                    != absent_body.to_string().replace(&nowhere, "<id>")
                || body.to_string().contains(QUIET_NAME)
            {
                named.push(format!(
                    "{quiet}: {status} {body} (no such conversation: {absent_status} {absent_body})"
                ));
            }
        }
        // Writes on the thread are answered the same way.
        for request in [
            AxumTestRequest::post(&format!("/api/chat/conversations/{}/read", w.quiet_thread)),
            AxumTestRequest::put(&format!("/api/chat/conversations/{}", w.quiet_thread))
                .json(&json!({ "title": "renamed" })),
            AxumTestRequest::post(&format!(
                "/api/chat/conversations/{}/messages",
                w.quiet_thread
            ))
            .json(&json!({ "content": "How is she doing?" })),
            AxumTestRequest::post(&format!(
                "/api/chat/conversations/{}/subject-plan/session-preview",
                w.quiet_thread
            ))
            .json(&json!({ "date": "2026-11-18", "family": "vo2max_short", "sport": "ride" })),
        ] {
            let (status, body) = w.rest(request, &w.api_key).await;
            if status != StatusCode::NOT_FOUND || body.to_string().contains(QUIET_NAME) {
                named.push(format!("a write: {status} {body}"));
            }
        }
        assert!(
            named.is_empty(),
            "bug: an outside client is told of the thread about a member who does not share:\n{}",
            named.join("\n")
        );

        // The thread about the member who shares still answers the client.
        let (status, body) = w
            .get(
                &format!("/api/chat/conversations/{}", w.shared_thread),
                &w.api_key,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["subject_display_name"], SHARING_NAME, "{body}");
    }

    #[tokio::test]
    async fn a_route_keyed_by_the_member_answers_an_outside_client_as_for_a_non_member() {
        let w = World::new().await;
        let stranger = person(&w.res, "stranger@outside-client-chat.test", "Sam Stranger").await;

        for path in ["thread", "plan", "physiology"] {
            let request = |athlete: Uuid| match path {
                "thread" => AxumTestRequest::post(&format!("/api/chat/athletes/{athlete}/thread"))
                    .json(&json!({ "group_id": w.group })),
                _ => AxumTestRequest::get(&format!(
                    "/api/chat/athletes/{athlete}/{path}?group_id={}",
                    w.group
                )),
            };
            let (status, body) = w.rest(request(w.quiet.id), &w.api_key).await;
            let (stranger_status, stranger_body) = w.rest(request(stranger.id), &w.api_key).await;
            assert_eq!(
                (status, &body),
                (stranger_status, &stranger_body),
                "bug: {path}: a member who stopped sharing is told apart from a non-member"
            );
            assert!(
                !body.to_string().contains(QUIET_NAME),
                "bug: {path}: the refusal names the member: {body}"
            );
        }
    }

    /// The group's room names every member to the coach's own app, and
    /// never the member who stopped sharing to an outside client acting for
    /// the coach.
    #[tokio::test]
    async fn the_room_never_names_the_member_to_an_outside_client() {
        let w = World::new().await;
        let transcript = format!("/api/chat/groups/{}/transcript", w.group);

        let (status, body) = w.get(&transcript, &w.api_key).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.to_string().contains(SHARING_NAME),
            "the member who shares is in the room: {body}"
        );
        assert!(
            !body.to_string().contains(QUIET_NAME)
                && !body.to_string().contains(&w.quiet.id.to_string()),
            "bug: the room names the member who stopped sharing to an outside client: {body}"
        );

        let (status, body) = w.get(&transcript, &w.coach.auth).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.to_string().contains(QUIET_NAME),
            "the coach's own app lists every member: {body}"
        );
    }

    /// The group's roster, the same: every member to the coach's own app,
    /// never the member who stopped sharing to an outside client.
    #[tokio::test]
    async fn the_roster_never_names_the_member_to_an_outside_client() {
        let w = World::new().await;
        let members = format!("/api/groups/{}/members", w.group);

        let (status, body) = w.group_get(&members, &w.api_key).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.to_string().contains(SHARING_NAME), "{body}");
        assert!(
            !body.to_string().contains(QUIET_NAME)
                && !body.to_string().contains(&w.quiet.id.to_string()),
            "bug: the roster names the member who stopped sharing to an outside client: {body}"
        );
        assert_eq!(
            body["total"],
            body["members"].as_array().unwrap().len(),
            "{body}"
        );

        let (status, body) = w.group_get(&members, &w.coach.auth).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.to_string().contains(QUIET_NAME),
            "the coach's own app lists every member: {body}"
        );
    }

    /// The coach's roster names every athlete to the coach's own app, and
    /// neither lists nor counts the one who stopped sharing to an outside
    /// client acting for the coach.
    #[tokio::test]
    async fn the_coach_roster_never_names_the_member_to_an_outside_client() {
        let w = World::new().await;
        let router = roster::router(&w.res);

        let (status, body) = roster::get(&router, "/api/coach/roster", &w.api_key).await;
        assert_eq!(status, 200, "{body}");
        assert!(body.to_string().contains(SHARING_NAME), "{body}");
        assert!(
            !w.names_quiet(&body),
            "bug: the coach roster names the member who stopped sharing to an outside client: {body}"
        );
        assert_eq!(body["scope"]["roster_members"], 1, "{body}");
        assert_eq!(body["groups"][0]["athlete_count"], 1, "{body}");

        let (status, body) = roster::get(&router, "/api/coach/roster", &w.coach.auth).await;
        assert_eq!(status, 200, "{body}");
        assert!(
            body.to_string().contains(QUIET_NAME),
            "the coach's own app lists every athlete: {body}"
        );
        assert_eq!(body["scope"]["roster_members"], 2, "{body}");
        assert_eq!(body["groups"][0]["athlete_count"], 2, "{body}");
    }

    /// A dismissal keyed by the member answers an outside client as for an
    /// athlete not on the roster.
    #[tokio::test]
    async fn an_attention_dismissal_answers_an_outside_client_as_for_a_non_member() {
        let w = World::new().await;
        let stranger = person(&w.res, "stranger@outside-client-chat.test", "Sam Stranger").await;
        let router = roster::router(&w.res);
        let dismiss = |athlete: Uuid| json!({ "athlete_user_id": athlete, "reason_id": "no_session", "fingerprint": "f" });

        let quiet = roster::post(
            &router,
            "/api/coach/attention/dismiss",
            &w.api_key,
            &dismiss(w.quiet.id),
        )
        .await;
        let mut absent = roster::post(
            &router,
            "/api/coach/attention/dismiss",
            &w.api_key,
            &dismiss(stranger.id),
        )
        .await;
        let mut quiet_body = quiet.1.clone();
        for body in [&mut quiet_body, &mut absent.1] {
            if let Some(error) = body.as_object_mut() {
                error.remove("timestamp");
            }
        }
        assert_eq!(
            (quiet.0, quiet_body),
            absent,
            "bug: a dismissal tells an outside client the member who stopped sharing is on the roster"
        );

        let (status, body) = roster::post(
            &router,
            "/api/coach/attention/dismiss",
            &w.coach.auth,
            &dismiss(w.quiet.id),
        )
        .await;
        assert_eq!(
            status, 200,
            "the coach's own app dismisses for any athlete: {body}"
        );
    }

    /// The reply review's athlete filter never names the member to an
    /// outside client, and a page filtered on her reads as one filtered on a
    /// non-member.
    #[tokio::test]
    async fn the_reply_review_never_names_the_member_to_an_outside_client() {
        let w = World::new().await;
        let stranger = person(&w.res, "stranger@outside-client-chat.test", "Sam Stranger").await;

        let (status, body) = w.replies_get("/api/coach/replies", &w.api_key).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.to_string().contains(SHARING_NAME), "{body}");
        assert!(
            !w.names_quiet(&body),
            "bug: the reply review names the member who stopped sharing to an outside client: {body}"
        );

        let filtered = w
            .replies_get(
                &format!("/api/coach/replies?athlete_id={}", w.quiet.id),
                &w.api_key,
            )
            .await;
        let absent = w
            .replies_get(
                &format!("/api/coach/replies?athlete_id={}", stranger.id),
                &w.api_key,
            )
            .await;
        assert_eq!(
            filtered, absent,
            "bug: a page filtered on the member is told apart from one filtered on a non-member"
        );

        let (status, body) = w.replies_get("/api/coach/replies", &w.coach.auth).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.to_string().contains(QUIET_NAME),
            "the coach's own app lists every athlete: {body}"
        );
    }

    /// `/group members` typed in the room never names the member to an
    /// outside client acting for the coach, nor counts her; the coach's own
    /// app is listed every member.
    #[tokio::test]
    async fn group_members_never_names_the_member_to_an_outside_client() {
        let w = World::new().await;

        let members_listed = |authorization: String| {
            let w = &w;
            async move {
                let (status, room) = w
                    .rest(
                        AxumTestRequest::post(&format!("/api/chat/groups/{}/room", w.group)),
                        &authorization,
                    )
                    .await;
                assert!(
                    status.is_success(),
                    "fixture: the room opens: {status} {room}"
                );
                let room = room["id"].as_str().unwrap().to_owned();
                let (status, turn) = w
                    .rest(
                        AxumTestRequest::post(&format!("/api/chat/conversations/{room}/messages"))
                            .json(&json!({ "content": "/group members" })),
                        &authorization,
                    )
                    .await;
                assert_eq!(status, StatusCode::OK, "{turn}");
                assert_eq!(turn["assistant"]["finish_reason"], "command", "{turn}");
                turn["assistant"]["message"]["content"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            }
        };

        let listed = members_listed(w.api_key.clone()).await;
        assert!(listed.contains(SHARING_NAME), "{listed}");
        assert!(
            !listed.contains(QUIET_NAME),
            "bug: /group members names the member who stopped sharing to an outside client:\n{listed}"
        );
        assert!(
            listed.contains("(1)"),
            "bug: the header counts a member the client is not listed:\n{listed}"
        );

        let listed = members_listed(w.coach.auth.clone()).await;
        assert!(
            listed.contains(QUIET_NAME) && listed.contains("(2)"),
            "the coach's own app lists every member:\n{listed}"
        );
    }

    /// Dravr's own app is unchanged: the coach opens the thread and is told,
    /// by name, that the member stopped sharing.
    #[tokio::test]
    async fn the_coach_in_dravr_still_opens_the_thread_and_is_told_by_name() {
        let w = World::new().await;

        let (status, body) = w
            .get(
                &format!("/api/chat/conversations/{}", w.quiet_thread),
                &w.coach.auth,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["subject_display_name"], QUIET_NAME, "{body}");

        let (status, body) = w
            .get(
                &format!("/api/chat/conversations/{}/subject-plan", w.quiet_thread),
                &w.coach.auth,
            )
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(
            body.to_string().contains(QUIET_NAME)
                && body.to_string().contains("coach_sharing_revoked"),
            "the coach is told who stopped sharing: {body}"
        );
    }
}
