// ABOUTME: /help is the cross-channel menu — a card with native buttons, not a wall of text
// ABOUTME: and in a shared room it marks the commands that act on one athlete alone

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! There is deliberately no `/menu` command. `/help` already owns the command
//! listing, and a second command computing the same list from the same
//! catalogue is the parallel system CLAUDE.md forbids — so `/help` is the
//! menu, and it comes back as a card whose buttons each channel renders
//! natively (a Telegram inline keyboard, Slack Block Kit, `WhatsApp` reply
//! buttons, a Messenger template).
//!
//! The assertions are on content, not on `is_ok()`: a handler that quietly
//! stopped returning a card, or stopped marking personal commands, would
//! still answer successfully with a plausible-looking body.

use anyhow::Result;
use pierre_commands::help::{HelpHandler, PERSONAL_MARKER};
use pierre_commands::{load_command_catalog, CommandHandler, PlatformCommandContext};
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_messaging::commands::{CommandRegistry, CommandResponse};
use pierre_runtime_context::CoachesCtx;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let email = format!("help_menu_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(resources.database(), &email).await?;
    let tenants = resources.common.repos.tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .map(|t| t.id)
        .expect("user should own a tenant");
    Ok((resources, user_id, tenant))
}

/// Build the handler over the repository's real `commands/` catalogue, so the
/// menu under test is the menu an athlete would actually receive.
fn handler() -> HelpHandler {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("commands");
    let catalog = load_command_catalog(&root);
    assert!(
        catalog.definitions.len() > 10,
        "the commands/ catalogue should have loaded from {}",
        root.display()
    );
    assert!(
        !catalog.personal.is_empty(),
        "the catalogue should mark some commands personal; found none"
    );
    let mut registry = CommandRegistry::new();
    for def in catalog.definitions {
        registry.register(def);
    }
    HelpHandler::new(
        Arc::new(registry),
        Arc::new(catalog.arg_specs),
        Arc::new(HashMap::new()),
        catalog.personal,
    )
}

fn ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    is_direct_message: bool,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/help".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message,
        ambient_group_fallback: true,
        conversation_id: None,
        conversation_tenant_id: tenant_id,
        sender_id: None,
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

async fn run(is_direct_message: bool) -> Result<CommandResponse> {
    let (resources, user_id, tenant) = setup().await?;
    Ok(handler()
        .execute(&ctx(&resources, user_id, tenant, is_direct_message))
        .await?)
}

#[tokio::test]
async fn help_comes_back_as_a_card_with_tappable_shortcuts() -> Result<()> {
    let response = run(true).await?;

    assert!(
        response.is_card(),
        "/help must be a card so every channel renders native buttons; \
         title={:?} actions={}",
        response.card_title,
        response.actions.len()
    );
    assert!(
        !response.actions.is_empty() && response.actions.len() <= 3,
        "a card must carry 1-3 buttons — three is the smallest cap any channel \
         allows — got {}",
        response.actions.len()
    );
    for action in &response.actions {
        // A postback value is the text the press stands for, so tapping the
        // button must be exactly typing the command.
        assert_eq!(
            action.action_type, "postback",
            "shortcut {:?} must be a postback, not a link",
            action.label
        );
        assert!(
            action.value.starts_with('/'),
            "shortcut {:?} must carry a real command, got {:?}",
            action.label,
            action.value
        );
        assert_eq!(
            action.label, action.value,
            "the button label is the command it runs, so the two cannot drift"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_shared_room_marks_the_commands_that_act_on_one_athlete() -> Result<()> {
    let response = run(false).await?;
    let body = &response.text;

    // /calibrate acts on the reader alone and must be flagged as such.
    let marked_line = body
        .lines()
        .find(|l| l.contains("/calibrate"))
        .unwrap_or_else(|| panic!("/calibrate missing from the listing:\n{body}"));
    assert!(
        marked_line.trim_start().starts_with(PERSONAL_MARKER),
        "/calibrate acts on one athlete and must be marked in a room: {marked_line:?}"
    );

    // ...and a command that acts on the ROOM must not be, or the marker
    // means nothing.
    let unmarked_line = body
        .lines()
        .find(|l| l.contains("/group status"))
        .unwrap_or_else(|| panic!("/group status missing from the listing:\n{body}"));
    assert!(
        !unmarked_line.trim_start().starts_with(PERSONAL_MARKER),
        "/group status reports the room and must not be marked: {unmarked_line:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_direct_message_carries_no_marker() -> Result<()> {
    let response = run(true).await?;
    assert!(
        !response.text.contains(PERSONAL_MARKER),
        "in a DM every command is the reader's own, so the marker is noise:\n{}",
        response.text
    );
    Ok(())
}

#[tokio::test]
async fn the_listing_still_carries_every_command_in_a_room() -> Result<()> {
    // Marking replaced hiding: a shared room must still be able to discover
    // the whole vocabulary, including the personal half.
    let room = run(false).await?;
    for expected in [
        "/calibrate",
        "/pillars",
        "/logout",
        "/plan",
        "/group",
        "/help",
    ] {
        assert!(
            room.text.contains(expected),
            "{expected} must stay discoverable in a shared room:\n{}",
            room.text
        );
    }
    Ok(())
}
