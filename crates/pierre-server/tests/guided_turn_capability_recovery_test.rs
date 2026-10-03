// ABOUTME: Capability recovery leaves an interview's answer alone, and frames its re-ask as evidence
// ABOUTME: Real turns through the chat pipeline against a recording model and a stand-in scraper

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live 2026-09-30, web chat, `/season` walk: after two of the athlete's
//! answers the delivered reply opened «Je n'ai pas reçu ta réponse à ma
//! question» and asked again. The agent's first reply — the right one — was
//! never shown.
//!
//! Capability recovery had fired on the answers. « des courses de trail
//! (25-65 km) » matched the data-ask vocabulary on a turn the walk runs
//! without the activity prefetch, so the turn looked ungrounded; the stage
//! fetched `get_activities` and re-asked with the dump appended as a `user`
//! message and the draft nowhere in the list. To the model the athlete's
//! reply to its question was a data dump — an athlete who never answered.
//!
//! Both halves are pinned here: an interview turn is never re-asked, and a
//! re-ask that does run carries the draft as the assistant's turn and the
//! fetched data under the platform's own frame.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

#[cfg(feature = "provider-sciotte")]
mod guided_turn {
    use pierre_core::transport::Transport;
    use std::env;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::Utc;
    use futures_util::stream;
    use serial_test::serial;
    use std::time::Duration;
    use tokio::time::sleep;
    use uuid::Uuid;

    use crate::common::{create_test_server_resources_with_llm, create_test_user_with_plan};
    use crate::helpers::sciotte_mock::{seed_sciotte_session, spawn_mock_scraper};
    use pierre_chat_pipeline::stages::capability_recovery::REPAIR_EVIDENCE_FRAME;
    use pierre_chat_pipeline::{
        CommandPersistence, PipelineHooks, ServedTurn, SurfaceId, SurfaceProfile, SurfaceRequest,
        TurnOrigin, TurnRequest,
    };
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatMessage, ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider,
        MessageRole, StreamChunk, TokenUsage,
    };
    use pierre_core::models::{
        AddMessageParams, ConnectionType, ConversationTurnId, GuidedFlow, OnboardingState, Pillar,
        SeasonTopic, TenantId, COMMAND_FINISH_REASON,
    };
    use pierre_database::repositories::UpsertUserFactParams;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};

    /// A substring of the capability re-ask's instruction: a request carrying
    /// it is the repair completion, not the turn's own.
    const REASK_MARKER: &str = "fetched successfully on your behalf";

    /// Heads the season walk's directive, which every coaching call of a
    /// `/season` turn carries and no background pass does.
    const SEASON_CALL_MARKER: &str = "# Season mode";

    /// The athlete's answer from the incident: it names races ("course"), a
    /// distance range and a month, and it answers the agent's question.
    const SEASON_ANSWER: &str =
        "Je fais surtout des courses de trail (25-65 km), et le VTXL en juin.";

    /// The agent's next question — the reply the athlete must receive.
    const SEASON_PROBE: &str =
        "Super, le trail c'est ton terrain ! Parmi ces courses, laquelle est ta priorité de \
         la saison, celle autour de laquelle on construit tout le reste ?";

    /// What the re-ask answers; seeing it delivered means a reply was replaced.
    const REASKED_REPLY: &str =
        "D'après ta sortie vélo matinale (21 km, 250 m), on part d'une base solide.";

    /// The athlete's earlier `/season` answer: their race calendar.
    const CALENDAR_ANSWER: &str =
        "Mon gros objectif c'est l'ultra de 150 km le 5 juin 2027, et sinon des courses de \
         trail de 25-65 km.";

    /// The agent's question after it, quoting the calendar back — the 15:12
    /// turn's previous reply, which the plain digit count read as a claim.
    const QUOTING_QUESTION: &str =
        "Noté : 150 km (5 juin 2027) comme course A, et des trails de 25–65 km autour. \
         Laquelle de ces courses vient en premier dans ta saison ?";

    /// A walk question that invents training volume the athlete never gave,
    /// with no tool behind it.
    const FABRICATING_QUESTION: &str =
        "Le mois dernier tu as couru 161 km avec 2391 m de dénivelé en 6,2h, belle base ! \
         Quelle est ta course A ?";

    /// A goal the athlete recorded in an earlier walk, held in the dossier.
    const DOSSIER_GOAL: &str = "Ultra-Trail des Laurentides, 150 km, 2027-06-05";

    /// A walk question restating that goal with the date written out.
    const DOSSIER_QUESTION: &str =
        "Ton ultra de 150 km le 5 juin 2027 reste ta course A, on bâtit la saison autour ?";

    /// The athlete disputing it.
    const DISPUTE: &str = "C'est faux, j'ai jamais fait ça le mois dernier.";

    /// A fabricated access-failure claim: the trigger that re-asks on any
    /// turn, grounded or not.
    const FABRICATED_CLAIM: &str =
        "Je ne suis pas capable d'accéder à tes données d'activité en ce moment (problème de \
         connexion de mon côté).";

    const DATA_ASK: &str = "Propose-moi une sortie basée sur mes activités récentes";

    /// Records every request's messages and answers the turn's own call with
    /// the scripted reply, the re-ask with [`REASKED_REPLY`].
    struct RecordingProvider {
        requests: Mutex<Vec<Vec<ChatMessage>>>,
        reply: String,
    }

    impl RecordingProvider {
        fn requests(&self) -> Vec<Vec<ChatMessage>> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn is_reask(messages: &[ChatMessage]) -> bool {
        messages.iter().any(|m| m.content.contains(REASK_MARKER))
    }

    #[async_trait]
    impl LlmProvider for RecordingProvider {
        fn name(&self) -> &'static str {
            "recording_mock"
        }
        fn display_name(&self) -> &'static str {
            "Recording Mock LLM (guided-turn capability recovery)"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }

        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
            self.requests.lock().unwrap().push(request.messages.clone());
            let content = if is_reask(&request.messages) {
                REASKED_REPLY.to_owned()
            } else {
                self.reply.clone()
            };
            Ok(ChatResponse {
                content,
                model: "mock-model".to_owned(),
                usage: Some(TokenUsage::new(30, 40, 70)),
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }

        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            let chunk = StreamChunk {
                delta: String::new(),
                is_final: true,
                finish_reason: Some("stop".to_owned()),
            };
            Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
        }

        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    struct Fixture {
        resources: Arc<ServerContext>,
        tenant_id: TenantId,
        user_id: Uuid,
        provider: Arc<RecordingProvider>,
        conversation_id: String,
    }

    /// An athlete whose sciotte connection serves one ride through the
    /// stand-in scraper, so a verification fetch succeeds and a re-ask runs
    /// whenever the stage decides it should.
    async fn setup(email: &str, reply: &str) -> Fixture {
        env::set_var("PIERRE_LLM_MODEL", "mock-model");
        let scraper_url = spawn_mock_scraper().await;
        env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
        // Both-or-neither: a URL with no audience disables the remote client.
        env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-test");

        let provider = Arc::new(RecordingProvider {
            requests: Mutex::new(Vec::new()),
            reply: reply.to_owned(),
        });
        let resources = create_test_server_resources_with_llm(provider.clone())
            .await
            .unwrap();
        let (user_id, _user, tenant_id) =
            create_test_user_with_plan(&resources.agent.database, email, "professional")
                .await
                .unwrap();
        resources
            .common
            .repos
            .provider_connections
            .register_connection(user_id, tenant_id, "sciotte", &ConnectionType::Manual, None)
            .await
            .unwrap();
        seed_sciotte_session(&resources, user_id, tenant_id).await;
        let conversation_id = resources
            .common
            .repos
            .chat
            .create_conversation(
                &user_id.to_string(),
                tenant_id,
                "Saison",
                "mock-model",
                None,
                None,
            )
            .await
            .unwrap()
            .id;
        Fixture {
            resources,
            tenant_id,
            user_id,
            provider,
            conversation_id,
        }
    }

    /// Run one athlete turn on the web surface and return the delivered reply.
    async fn run_turn(fx: &Fixture, content: &str) -> String {
        let served = pierre_chat_pipeline::execute(
            &fx.resources.chat_pipeline_context(),
            TurnRequest {
                origin: TurnOrigin::Athlete,
                conversation_id: fx.conversation_id.clone(),
                user_id: fx.user_id,
                conversation_tenant_id: fx.tenant_id,
                tool_tenant_id: fx.tenant_id,
                content: content.to_owned(),
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                channel_type: "web",
                transport: Transport::WebApp,
                is_direct_message: true,
                ambient_group_fallback: false,
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                hooks: PipelineHooks::none(),
            },
            &SurfaceProfile::resolve(&SurfaceRequest {
                surface: SurfaceId::Web,
                locale: "fr".to_owned(),
                transport: None,
                prose_contract: None,
            }),
        )
        .await
        .expect("the turn is served");
        match served {
            ServedTurn::Pipeline(envelope) => envelope.assistant.message.content,
            ServedTurn::Command { .. } => panic!("an athlete's message must reach the pipeline"),
        }
    }

    /// Open a `/season` walk on the fixture's conversation, with `probed`
    /// the walk questions that already reached the athlete — empty on the
    /// walk's first answer, which replies to the platform's opener.
    async fn start_season_walk(fx: &Fixture, probed: &[SeasonTopic]) {
        let mut state = OnboardingState::start(Utc::now().to_rfc3339(), GuidedFlow::Season);
        state.probed = probed.iter().map(|topic| topic.slug()).collect();
        let walk = state.to_column().unwrap();
        assert!(fx
            .resources
            .common
            .repos
            .chat
            .set_conversation_onboarding_state(&fx.conversation_id, Some(&walk), fx.tenant_id)
            .await
            .unwrap());
    }

    /// Write earlier walk exchanges into the transcript, oldest first.
    async fn seed_history(fx: &Fixture, rows: &[(&str, &str)]) {
        seed_rows(fx, rows, None).await;
    }

    /// [`seed_history`] with every row stamped `finish_reason`.
    async fn seed_rows(fx: &Fixture, rows: &[(&str, &str)], finish_reason: Option<&str>) {
        let user_id = fx.user_id.to_string();
        for (role, content) in rows {
            fx.resources
                .common
                .repos
                .chat
                .add_message(&AddMessageParams {
                    tenant_id: fx.tenant_id,
                    conversation_id: &fx.conversation_id,
                    user_id: &user_id,
                    role,
                    content,
                    token_count: None,
                    finish_reason,
                    prompt_tokens: None,
                    model: None,
                    content_blocks: None,
                })
                .await
                .unwrap();
            // Distinct timestamps, so the transcript replays in this order.
            sleep(Duration::from_millis(15)).await;
        }
    }

    /// Completions that carried the season walk's directive.
    fn season_calls(fx: &Fixture) -> usize {
        fx.provider
            .requests()
            .iter()
            .filter(|messages| {
                messages.iter().any(|m| {
                    m.role == MessageRole::System && m.content.contains(SEASON_CALL_MARKER)
                })
            })
            .count()
    }

    /// A walk's first answer replies to the platform's opener — a command turn
    /// kept out of replay. The last assistant message the model sees is then
    /// the coaching reply from before the walk, whose figures came from a
    /// prefetch that was never persisted. That reply is not the one the
    /// athlete is answering, so it is not re-grounded: one completion.
    #[tokio::test]
    #[serial]
    async fn a_walks_first_answer_does_not_judge_the_reply_before_the_walk() {
        let fx = setup("guided-recovery-opener@test.com", SEASON_PROBE).await;
        seed_history(
            &fx,
            &[
                ("user", "Comment était ma semaine ?"),
                (
                    "assistant",
                    "Belle semaine : 161 km, 2391 m de dénivelé, 6,2 h de selle.",
                ),
            ],
        )
        .await;
        seed_rows(
            &fx,
            &[
                ("user", "/season"),
                ("assistant", "Six questions courtes pour cadrer ta saison."),
            ],
            Some(COMMAND_FINISH_REASON),
        )
        .await;
        start_season_walk(&fx, &[]).await;

        let delivered = run_turn(&fx, "Je fais surtout du trail.").await;

        assert_eq!(
            season_calls(&fx),
            1,
            "the pre-walk reply is not the one the first answer replies to"
        );
        assert!(!fx.provider.requests().iter().any(|m| is_reask(m)));
        assert_eq!(delivered, SEASON_PROBE);
    }

    /// The 15:12 shape: the agent's previous question quoted the athlete's own
    /// calendar back — five numbers, every one of them the athlete's. That is
    /// not a claim about their training, so the next answer is one call too.
    #[tokio::test]
    #[serial]
    async fn a_walk_question_quoting_the_athletes_calendar_is_not_a_claim() {
        let fx = setup("guided-recovery-quoted@test.com", SEASON_PROBE).await;
        start_season_walk(&fx, &[]).await;
        seed_history(
            &fx,
            &[("user", CALENDAR_ANSWER), ("assistant", QUOTING_QUESTION)],
        )
        .await;

        let delivered = run_turn(&fx, "Le VTXL en juin, c'est ma première.").await;

        assert_eq!(
            season_calls(&fx),
            1,
            "a quoted-back calendar is not re-grounded"
        );
        assert!(!fx.provider.requests().iter().any(|m| is_reask(m)));
        assert_eq!(delivered, SEASON_PROBE);
    }

    /// The goal the walk quotes back lives in the dossier, not in this
    /// conversation, and the question writes its date another way. The
    /// dossier is context the model was handed, and a date is compared by
    /// value, so the answer after it is still one call.
    #[tokio::test]
    #[serial]
    async fn a_walk_question_restating_the_dossier_is_not_a_claim() {
        let fx = setup("guided-recovery-dossier@test.com", SEASON_PROBE).await;
        fx.resources
            .common
            .repos
            .memory
            .upsert_user_fact(&UpsertUserFactParams {
                tenant_id: fx.tenant_id,
                user_id: &fx.user_id.to_string(),
                agent_id: None,
                scope: MemoryScope::User,
                kind: FactKind::Goal,
                pillar: Some(Pillar::TrainingAndMovement),
                predicate_code: PredicateCode::TrainingFor,
                object: DOSSIER_GOAL,
                confidence: 0.9,
                source: FactSource::Onboarding,
                valid_until: None,
                source_msg_id: None,
            })
            .await
            .unwrap();
        start_season_walk(&fx, &[SeasonTopic::RaceCalendar]).await;
        seed_history(&fx, &[("user", "Salut !"), ("assistant", DOSSIER_QUESTION)]).await;

        let delivered = run_turn(&fx, "Oui, c'est toujours elle.").await;

        let season_prompts: Vec<String> = fx
            .provider
            .requests()
            .iter()
            .filter_map(|messages| {
                messages
                    .iter()
                    .find(|m| {
                        m.role == MessageRole::System && m.content.contains(SEASON_CALL_MARKER)
                    })
                    .map(|m| m.content.clone())
            })
            .collect();
        assert_eq!(
            season_prompts.len(),
            1,
            "a dossier restatement is not re-grounded"
        );
        assert!(
            season_prompts[0].contains("2027-06-05"),
            "the goal must reach the model through the dossier, or this test proves nothing"
        );
        assert!(!fx.provider.requests().iter().any(|m| is_reask(m)));
        assert_eq!(delivered, SEASON_PROBE);
    }

    /// Small invented figures are still claims when they happen to match the
    /// system prompt's clock: a question dated today, with 30 km and 9 rides
    /// the athlete never gave, is re-grounded when disputed. Only the
    /// dossier's facts in the prompt are the athlete's.
    #[tokio::test]
    #[serial]
    async fn small_invented_figures_matching_the_prompt_clock_are_regrounded() {
        let fx = setup("guided-recovery-clock@test.com", SEASON_PROBE).await;
        start_season_walk(&fx, &[SeasonTopic::RaceCalendar]).await;
        let today = chrono::Utc::now().date_naive();
        let question = format!(
            "Tu fais déjà 30 km, 9 sorties, et ta course est le {}. Laquelle vises-tu ?",
            today.format("%-d/%-m/%Y")
        );
        seed_history(
            &fx,
            &[
                ("user", "Je fais surtout du trail."),
                ("assistant", &question),
            ],
        )
        .await;

        let delivered = run_turn(&fx, DISPUTE).await;

        let reasks = fx
            .provider
            .requests()
            .iter()
            .filter(|messages| is_reask(messages))
            .count();
        assert_eq!(
            reasks, 1,
            "30, 9 and today's date are the prompt's, not the athlete's"
        );
        assert_eq!(delivered, REASKED_REPLY);
    }

    /// Mid-walk, the agent asserts training volume the athlete never gave,
    /// with no tool call; the athlete disputes it. That is what
    /// `DisputedClaims` exists for, walk or not — recovery fires once, with
    /// the dispute as the athlete's message and the question as the draft.
    #[tokio::test]
    #[serial]
    async fn fabricated_volume_disputed_mid_walk_is_regrounded() {
        let fx = setup("guided-recovery-fabricated@test.com", SEASON_PROBE).await;
        start_season_walk(&fx, &[SeasonTopic::RaceCalendar]).await;
        seed_history(
            &fx,
            &[
                ("user", "Je fais surtout du trail."),
                ("assistant", FABRICATING_QUESTION),
            ],
        )
        .await;

        let delivered = run_turn(&fx, DISPUTE).await;

        let reasks: Vec<Vec<ChatMessage>> = fx
            .provider
            .requests()
            .into_iter()
            .filter(|messages| is_reask(messages))
            .collect();
        assert_eq!(
            reasks.len(),
            1,
            "an invented figure the athlete disputes is re-grounded"
        );
        let messages = &reasks[0];
        let n = messages.len();
        assert!(n >= 4, "got {n} messages");
        assert_eq!(messages[n - 1].role, MessageRole::User);
        assert!(messages[n - 1].content.starts_with(REPAIR_EVIDENCE_FRAME));
        assert!(messages[n - 1].content.contains("Sortie vélo matinale"));
        assert_eq!(messages[n - 2].role, MessageRole::Assistant);
        assert_eq!(messages[n - 2].content, SEASON_PROBE);
        assert_eq!(messages[n - 3].role, MessageRole::User);
        assert!(
            messages[n - 3].content.contains(DISPUTE),
            "the dispute stays the athlete's last message, got: {:?}",
            messages[n - 3].content
        );
        assert_eq!(delivered, REASKED_REPLY);
    }

    /// The incident turn: a `/season` answer naming races on a turn the walk
    /// runs without the prefetch. The agent's question is delivered as
    /// written, from one completion, and no verification re-ask is made.
    #[tokio::test]
    #[serial]
    async fn a_season_answer_naming_races_keeps_the_agents_first_reply() {
        let fx = setup("guided-recovery-season@test.com", SEASON_PROBE).await;
        start_season_walk(&fx, &[SeasonTopic::RaceCalendar]).await;

        let delivered = run_turn(&fx, SEASON_ANSWER).await;

        let requests = fx.provider.requests();
        let coaching_calls = season_calls(&fx);
        assert_eq!(
            coaching_calls, 1,
            "a season answer is one completion — a second call carrying the walk's directive \
             is a re-ask replacing the agent's question"
        );
        assert!(
            !requests.iter().any(|messages| is_reask(messages)),
            "an interview turn must never be re-asked with fetched activities"
        );
        assert_eq!(
            delivered, SEASON_PROBE,
            "the athlete must receive the question the agent asked, not a re-ask's reply"
        );
    }

    /// A re-ask on an ordinary turn keeps the conversation's shape: the
    /// athlete's message is still the last thing the athlete said, the draft
    /// is the assistant's turn, and the fetched data arrives under the
    /// platform's frame rather than as the athlete speaking.
    #[tokio::test]
    #[serial]
    async fn a_reask_carries_the_draft_and_frames_the_data_as_evidence() {
        let fx = setup("guided-recovery-reask@test.com", FABRICATED_CLAIM).await;

        let delivered = run_turn(&fx, DATA_ASK).await;

        let reasks: Vec<Vec<ChatMessage>> = fx
            .provider
            .requests()
            .into_iter()
            .filter(|messages| is_reask(messages))
            .collect();
        assert_eq!(
            reasks.len(),
            1,
            "a disproven claim is re-asked exactly once"
        );
        let messages = &reasks[0];
        let n = messages.len();
        assert!(
            n >= 4,
            "system prompt, athlete, draft, evidence: got {n} messages"
        );

        let evidence = &messages[n - 1];
        assert_eq!(evidence.role, MessageRole::User);
        assert!(
            evidence.content.starts_with(REPAIR_EVIDENCE_FRAME),
            "the evidence turn must open by saying the athlete did not write it, got: {:?}",
            evidence.content.chars().take(160).collect::<String>()
        );
        assert!(
            evidence
                .content
                .contains("<tool_result name=\"get_activities\">"),
            "the fetched activities ride in the evidence turn as a tool result"
        );
        assert!(
            evidence.content.contains("Sortie vélo matinale"),
            "the stand-in scraper's ride is the evidence the re-ask is built on"
        );
        assert!(evidence.content.contains(REASK_MARKER));

        let draft = &messages[n - 2];
        assert_eq!(
            draft.role,
            MessageRole::Assistant,
            "the draft reply must be the assistant's turn"
        );
        assert_eq!(draft.content, FABRICATED_CLAIM);

        let athlete = &messages[n - 3];
        assert_eq!(
            athlete.role,
            MessageRole::User,
            "the athlete's message stays the one the draft answers"
        );
        assert!(
            athlete.content.contains(DATA_ASK),
            "the message before the draft must be the athlete's own, got: {:?}",
            athlete.content
        );
        assert!(
            !athlete.content.contains(REPAIR_EVIDENCE_FRAME),
            "the athlete's message must not be the platform's evidence"
        );

        assert_eq!(
            delivered, REASKED_REPLY,
            "the disproven claim is replaced by the re-ask's reply"
        );
    }
}
