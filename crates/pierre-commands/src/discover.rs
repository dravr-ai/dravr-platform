// ABOUTME: Handlers for /discover — browse or search the coach catalogue and install a coach by @handle
// ABOUTME: The chat face of the Coach Store, on the same reads and install path as the web tab and the tools
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_DISCOVER_ADD_LABEL, KEY_DISCOVER_CARD_TITLE, KEY_DISCOVER_CATALOGUE_EMPTY,
    KEY_DISCOVER_EMPTY, KEY_DISCOVER_INSTALLED, KEY_DISCOVER_INSTALL_ALREADY,
    KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, KEY_DISCOVER_INSTALL_USAGE, KEY_DISCOVER_ITEM,
    KEY_DISCOVER_MORE_LABEL,
};
use pierre_core::errors::AppError;
use pierre_core::markdown::strip_emphasis;
use pierre_core::models::coaches::{CoachCategory, CoachHandle};
use pierre_messaging::commands::{CommandAction, CommandResponse};
use pierre_services::coach_store::{
    browse_store_page, install_store_coach, search_store, StoreCoach,
};
use tracing::{debug, info};

use crate::{CommandHandler, PlatformCommandContext};

/// Coaches per card — one button each, so a card stays within every
/// channel's control budget and every button value within Telegram's
/// 64-byte callback limit (`/discover install @` plus a 40-character handle).
const PAGE_SIZE: u32 = 8;

/// The seven categories the Store files coaches under — the same set the web
/// Discover tab offers as filters.
const STORE_CATEGORIES: [CoachCategory; 7] = [
    CoachCategory::Training,
    CoachCategory::Nutrition,
    CoachCategory::Recovery,
    CoachCategory::Recipes,
    CoachCategory::Mobility,
    CoachCategory::Analysis,
    CoachCategory::Custom,
];

/// What the words after `/discover` ask for.
enum Scope {
    /// Bare `/discover`: the newest published coaches.
    All,
    /// One of the seven store categories, named exactly (case-insensitive).
    Category(CoachCategory),
    /// Anything else: a search over titles, descriptions and tags.
    Search(String),
}

impl Scope {
    /// Read the scope from the words that follow the command (and any
    /// `more <offset>` prefix). A category name wins over a search for the
    /// same word, so `/discover recovery` browses the Recovery shelf rather
    /// than searching for the word.
    fn parse(words: &[String]) -> Self {
        let text = words.join(" ");
        let text = text.trim();
        if text.is_empty() {
            return Self::All;
        }
        STORE_CATEGORIES
            .iter()
            .copied()
            .find(|category| category.as_str().eq_ignore_ascii_case(text))
            .map_or_else(|| Self::Search(text.to_owned()), Self::Category)
    }
}

/// Split a `more <offset>` prefix — the shape the card's "More" button
/// sends — off the arguments, leaving the scope words. An offset that does
/// not parse starts the browse over from the first page.
fn split_page(args: &[String]) -> (u32, &[String]) {
    match args {
        [first, offset, rest @ ..] if first.eq_ignore_ascii_case("more") => {
            (offset.parse().unwrap_or(0), rest)
        }
        _ => (0, args),
    }
}

/// What one `/discover` turn found.
struct Listing {
    /// The coaches to show, at most [`PAGE_SIZE`].
    coaches: Vec<StoreCoach>,
    /// The postback that fetches the next page, when there is one.
    next_page: Option<String>,
    /// What the caller asked for, for the empty reply — `None` for the bare
    /// catalogue.
    asked: Option<String>,
}

/// Handler for `/discover [query|category]` and `/discover more <offset> [category]`.
///
/// Reads the Store through the same service the web Discover tab and the
/// `browse_coach_store` / `search_coach_store` tools use, so a coach ranks
/// the same everywhere. Every coach on the card gets an install button;
/// only a browse pages, since a search already returns its best matches.
pub struct DiscoverHandler;

impl DiscoverHandler {
    async fn list(ctx: &PlatformCommandContext) -> Result<Listing, AppError> {
        let repos = ctx.ctx.repos().coach_repos();
        let (offset, words) = split_page(&ctx.args);
        Ok(match Scope::parse(words) {
            Scope::All => {
                let page =
                    browse_store_page(&repos, ctx.tenant_id, None, offset, PAGE_SIZE).await?;
                Listing {
                    coaches: page.coaches,
                    next_page: page
                        .next_offset
                        .map(|next| format!("/discover more {next}")),
                    asked: None,
                }
            }
            Scope::Category(category) => {
                let page =
                    browse_store_page(&repos, ctx.tenant_id, Some(category), offset, PAGE_SIZE)
                        .await?;
                Listing {
                    coaches: page.coaches,
                    next_page: page
                        .next_offset
                        .map(|next| format!("/discover more {next} {}", category.as_str())),
                    asked: Some(category.display_name().to_owned()),
                }
            }
            Scope::Search(query) => Listing {
                coaches: search_store(&repos, &query, Some(PAGE_SIZE)).await?,
                next_page: None,
                asked: Some(query),
            },
        })
    }
}

#[async_trait]
impl CommandHandler for DiscoverHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let listing = Self::list(ctx).await?;
        if listing.coaches.is_empty() {
            let body = listing.asked.map_or_else(
                || reg.render(KEY_DISCOVER_CATALOGUE_EMPTY, locale, &[]),
                |asked| reg.render(KEY_DISCOVER_EMPTY, locale, &[&asked]),
            );
            return Ok(CommandResponse::text(body));
        }

        let mut body = String::with_capacity(512);
        let mut actions = Vec::with_capacity(listing.coaches.len() + 1);
        for coach in &listing.coaches {
            // A handle is assigned when a coach is approved into the Store;
            // a listing without one predates that and is installable from
            // the web tab by id, but not by name from chat.
            let Some(handle) = coach.handle.as_deref() else {
                debug!(coach_id = %coach.id, "published coach has no catalogue handle; not listed in /discover");
                continue;
            };
            // Coach markdown carries CommonMark emphasis that only Slack
            // renders natively; strip it so every channel reads plain text,
            // and keep the first line — a catalogue card is a shelf, not the
            // coach's page.
            let description = coach
                .description
                .as_deref()
                .and_then(|d| d.lines().find(|line| !line.trim().is_empty()))
                .map(strip_emphasis)
                .unwrap_or_default();
            body.push_str(&reg.render(
                KEY_DISCOVER_ITEM,
                locale,
                &[
                    &coach.title,
                    handle,
                    coach.category.display_name(),
                    &description,
                ],
            ));
            actions.push(CommandAction {
                label: coach.title.clone(),
                action_type: "postback".to_owned(),
                value: format!("/discover install @{handle}"),
            });
        }
        if let Some(next_page) = listing.next_page {
            actions.push(CommandAction {
                label: reg.render(KEY_DISCOVER_MORE_LABEL, locale, &[]),
                action_type: "postback".to_owned(),
                value: next_page,
            });
        }

        Ok(CommandResponse::card(
            reg.render(KEY_DISCOVER_CARD_TITLE, locale, &[]),
            body,
            actions,
        ))
    }
}

/// Handler for `/discover install @handle`.
///
/// Resolves the handle against the published catalogue — the origin coach,
/// never an athlete's copy — and installs it through the one path the REST
/// route and the `install_coach_from_store` tool share, so `coach.installed`
/// counts the install once. The reply is the post-install hint: how to bring
/// the coach into a chat (`/coach add @handle`) or borrow it for one turn
/// (`@handle` in a message). A coach already on the caller's list gets the
/// same hint and no second copy.
pub struct DiscoverInstallHandler;

#[async_trait]
impl CommandHandler for DiscoverInstallHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let Some(typed) = ctx.args.first().map(|a| a.trim()).filter(|a| !a.is_empty()) else {
            return Ok(CommandResponse::text(reg.render(
                KEY_DISCOVER_INSTALL_USAGE,
                locale,
                &[],
            )));
        };
        // A token the handle grammar refuses names no published coach either;
        // both are answered by name.
        let Ok(handle) = CoachHandle::parse(typed) else {
            return Ok(unknown_handle(ctx, typed));
        };

        let repos = ctx.ctx.repos();
        if let Some(installed) = repos
            .coaches
            .find_installed_by_handle(&handle, ctx.user_id, ctx.tenant_id)
            .await?
        {
            return Ok(install_hint(
                ctx,
                KEY_DISCOVER_INSTALL_ALREADY,
                &installed.title,
                &handle,
            ));
        }

        let Some(published) = repos
            .store_listings
            .find_published_by_handle(&handle)
            .await?
        else {
            return Ok(unknown_handle(ctx, typed));
        };
        let installed = install_store_coach(
            &repos.coach_repos(),
            &published.coach.id.to_string(),
            ctx.user_id,
            ctx.tenant_id,
        )
        .await?;

        info!(
            user_id = %ctx.user_id,
            coach_id = %installed.id,
            handle = handle.as_str(),
            channel = %ctx.channel_type,
            "Coach installed from the catalogue via /discover install"
        );

        Ok(install_hint(
            ctx,
            KEY_DISCOVER_INSTALLED,
            &installed.title,
            &handle,
        ))
    }
}

/// The post-install card: the coach's title above the hint, and one button
/// that binds the coach to this chat.
fn install_hint(
    ctx: &PlatformCommandContext,
    key: &str,
    title: &str,
    handle: &CoachHandle,
) -> CommandResponse {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();
    CommandResponse::card(
        title.to_owned(),
        reg.render(key, locale, &[title, handle.as_str()]),
        vec![CommandAction {
            label: reg.render(KEY_DISCOVER_ADD_LABEL, locale, &[]),
            action_type: "postback".to_owned(),
            value: format!("/coach add @{}", handle.as_str()),
        }],
    )
}

/// Refuse a handle no published coach answers to, naming it as typed.
fn unknown_handle(ctx: &PlatformCommandContext, typed: &str) -> CommandResponse {
    let shown = if typed.starts_with('@') {
        typed.to_owned()
    } else {
        format!("@{typed}")
    };
    CommandResponse::text(ctx.ctx.messaging_strings_registry().render(
        KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE,
        ctx.locale.as_str(),
        &[&shown],
    ))
}
