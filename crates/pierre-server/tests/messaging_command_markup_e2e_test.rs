// ABOUTME: Command replies leave the messaging egress in canot's rich-text dialect, never as raw markdown
// ABOUTME: Drives /privacy and /help over the real wire and reads the ledgered body the channel was handed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

/// Command handlers answer in inline markdown, which the in-app surfaces parse
/// as is. A messaging channel cannot: its renderer translates canot's rich-text
/// dialect, so the egress converts the markdown before the channel sees it.
/// Both tests read the outbound ledger — the body the channel adapter was
/// handed — rather than the handler's text, because the conversion is the
/// egress's job and a test of the handler alone would pass with it missing.
#[cfg(feature = "client-messaging")]
mod egress {
    use crate::common::create_test_server_resources_with_chat_provider;
    use crate::helpers::command_e2e::{CommandE2e, Member, RouterLlm};
    use serial_test::serial;
    use std::env;
    use std::sync::Arc;

    /// A fresh linked member primed with `/status`, so the DM session the
    /// command replies are ledgered against exists. Returns the member, their
    /// session id and the ledger baseline.
    async fn primed_member(e2e: &CommandE2e) -> (Member, String, i64) {
        let member = e2e.linked_member(false).await;
        let prime = e2e.send_dm(&member, "/status").await;
        assert_eq!(
            prime.messages_stored(),
            0,
            "the /status prime must dispatch as a command"
        );
        let session = e2e
            .session_id(&member, member.home_tenant, &member.channel_user_id)
            .await
            .expect("the prime must forge a DM session");
        let baseline = e2e.wait_outbound_for_session(&session, 1).await;
        (member, session, baseline)
    }

    /// Send `command` and return the bodies ledgered after the baseline.
    async fn command_bodies(
        e2e: &CommandE2e,
        member: &Member,
        session: &str,
        baseline: i64,
        command: &str,
    ) -> Vec<String> {
        let ack = e2e.send_dm(member, command).await;
        assert_eq!(
            ack.messages_stored(),
            0,
            "{command} must dispatch as a command, not as a chat turn"
        );
        e2e.wait_outbound_for_session(session, baseline + 1).await;
        e2e.outbound_bodies_for_session(session)
            .await
            .into_iter()
            .skip(usize::try_from(baseline).unwrap())
            .collect()
    }

    async fn start() -> Arc<CommandE2e> {
        env::set_var("PIERRE_LLM_MODEL", "gemini-2.0-flash-exp");
        let llm = RouterLlm::new();
        let resources = create_test_server_resources_with_chat_provider(Arc::clone(&llm) as _)
            .await
            .unwrap();
        CommandE2e::start(resources, llm).await
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn privacy_reply_reaches_the_channel_in_its_dialect_not_as_markdown() {
        let e2e = start().await;
        let (member, session, baseline) = primed_member(&e2e).await;

        let bodies = command_bodies(&e2e, &member, &session, baseline, "/privacy").await;
        let body = bodies
            .iter()
            .find(|b| b.contains("/privacy on"))
            .unwrap_or_else(|| panic!("the /privacy status line must be ledgered: {bodies:?}"));

        assert!(
            body.contains("<b>") && body.contains("<code>/privacy on</code>"),
            "the status word and the command name must arrive as canot tags: {body}"
        );
        assert!(
            !body.contains("**") && !body.contains('`'),
            "no markdown marker may reach a channel adapter: {body}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[serial]
    async fn help_listing_reaches_the_channel_with_bold_headings_not_asterisks() {
        let e2e = start().await;
        let (member, session, baseline) = primed_member(&e2e).await;

        let bodies = command_bodies(&e2e, &member, &session, baseline, "/help").await;
        let body = bodies
            .iter()
            .find(|b| b.contains("\n- /"))
            .unwrap_or_else(|| panic!("the /help listing must be ledgered: {bodies:?}"));

        assert!(
            body.contains("<b>") && body.contains("</b>"),
            "a domain heading must arrive as a bold span, the Telegram `**Compte**` regression: {body}"
        );
        assert!(
            !body.contains("**"),
            "no heading may reach a channel adapter as literal asterisks: {body}"
        );
    }
}
