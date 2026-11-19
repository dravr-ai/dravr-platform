// ABOUTME: The startup menu publish sends the catalogue to the bot and the page it is handed, and to no one otherwise
// ABOUTME: Runs the publish the binary spawns against a local stand-in for the Bot API and the Graph API
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The production half of carnet#1600.
//!
//! Taking the menu publish out of the context constructor must not take it
//! out of the deployment: handed a bot and a page, the publish the server
//! binary runs at boot still makes every call it made, in the order it made
//! them. The stand-in below answers for both APIs on one local listener, so
//! nothing here reaches Telegram or Meta.

#[cfg(feature = "client-messaging")]
mod channel_menu_publish {
    use std::sync::{Arc, Mutex};

    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::Uri;
    use axum::Router;
    use pierre_core::models::SUPPORTED_LOCALES;
    use pierre_mcp_server::services::channel_menus::{channel_menu_publish, ChannelMenuTargets};
    use pierre_services::messenger_persistent_menu::MessengerPage;
    use pierre_services::telegram_bot_commands::{
        telegram_description_calls, TelegramBot, PERSONAL_MARKER,
    };
    use secrecy::SecretString;
    use serde_json::{json, Value};
    use tokio::net::TcpListener;

    use crate::common::create_test_server_resources;

    const BOT_TOKEN: &str = "0:test";
    const PAGE_TOKEN: &str = "test-page-token";

    /// Every request the stand-in received: path with query, and JSON body.
    type Received = Arc<Mutex<Vec<(String, Value)>>>;

    /// A stand-in for the Bot API and the Graph API that records each call
    /// and accepts it. Returns its origin.
    async fn channel_api_stand_in() -> (String, Received) {
        let received = Received::default();
        let app = Router::new()
            .fallback(
                |State(received): State<Received>, uri: Uri, body: Bytes| async move {
                    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    received.lock().unwrap().push((uri.to_string(), body));
                    axum::Json(json!({ "ok": true, "result": true }))
                },
            )
            .with_state(Arc::clone(&received));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (origin, received)
    }

    #[tokio::test]
    async fn a_named_bot_and_page_get_every_menu_call_in_order() {
        let (origin, received) = channel_api_stand_in().await;
        let resources = create_test_server_resources().await.unwrap();

        let targets = ChannelMenuTargets {
            telegram: Some(TelegramBot::at(&origin, SecretString::from(BOT_TOKEN))),
            messenger: Some(MessengerPage::at(&origin, SecretString::from(PAGE_TOKEN))),
        };
        channel_menu_publish(&resources, targets)
            .expect("a context with a command catalogue and a target publishes")
            .await;

        let received = received.lock().unwrap().clone();

        // The `/` menu: two lists per locale, then the two fallback lists.
        let menu_path = format!("/bot{BOT_TOKEN}/setMyCommands");
        let mut expected_menus: Vec<(Option<&str>, Value)> = Vec::new();
        for locale in SUPPORTED_LOCALES.into_iter().map(Some).chain([None]) {
            expected_menus.push((locale, Value::Null));
            expected_menus.push((locale, json!({ "type": "all_group_chats" })));
        }
        let menus = &received[..expected_menus.len().min(received.len())];
        assert_eq!(
            menus
                .iter()
                .map(|(path, body)| (
                    path.as_str(),
                    body["language_code"].as_str(),
                    body["scope"].clone()
                ))
                .collect::<Vec<_>>(),
            expected_menus
                .iter()
                .map(|(locale, scope)| (menu_path.as_str(), *locale, scope.clone()))
                .collect::<Vec<_>>(),
            "the / menu goes out as a default list and a group list per locale, \
             then the two with no language"
        );
        for (_, body) in menus {
            let commands = body["commands"].as_array().unwrap();
            assert!(!commands.is_empty(), "a published list carries commands");
            let marked = commands
                .iter()
                .filter(|c| {
                    c["description"]
                        .as_str()
                        .unwrap()
                        .starts_with(PERSONAL_MARKER)
                })
                .count();
            if body["scope"].is_null() {
                assert_eq!(marked, 0, "a direct chat's menu marks nothing");
            } else {
                assert!(
                    marked > 0,
                    "a shared room's menu marks the personal commands"
                );
            }
        }

        // Then the bot's description, exactly the calls the registry yields.
        let descriptions = telegram_description_calls(
            &resources.mcp.messaging_strings_registry,
            &SUPPORTED_LOCALES,
        );
        assert!(
            !descriptions.is_empty(),
            "the strings registry carries the bot description"
        );
        let after_menus = &received[menus.len()..];
        let sent_descriptions = &after_menus[..descriptions.len().min(after_menus.len())];
        assert_eq!(
            sent_descriptions
                .iter()
                .map(|(_, body)| body.clone())
                .collect::<Vec<_>>(),
            descriptions
                .iter()
                .map(|call| call.body.clone())
                .collect::<Vec<_>>(),
            "the bot description follows the menu, one call per text and language"
        );
        assert!(
            sent_descriptions.iter().all(|(path, _)| {
                path == &format!("/bot{BOT_TOKEN}/setMyDescription")
                    || path == &format!("/bot{BOT_TOKEN}/setMyShortDescription")
            }),
            "each description call names its own Bot API method"
        );

        // Messenger's persistent menu is the last call.
        let rest = &after_menus[sent_descriptions.len()..];
        assert_eq!(
            rest.len(),
            1,
            "one Messenger call ends the publish: {rest:?}"
        );
        let (path, body) = &rest[0];
        assert!(
            path.ends_with(&format!("/me/messenger_profile?access_token={PAGE_TOKEN}")),
            "the persistent menu is written to the page's Messenger profile, got {path}"
        );
        assert!(
            body["persistent_menu"][0]["call_to_actions"]
                .as_array()
                .is_some_and(|actions| !actions.is_empty()),
            "the persistent menu carries the catalogue"
        );
    }

    #[tokio::test]
    async fn a_context_handed_no_target_publishes_nothing() {
        let resources = create_test_server_resources().await.unwrap();
        // What a config the startup reader did not build yields.
        let targets = ChannelMenuTargets::from_config(&resources.common.config.channels);
        assert!(
            targets.telegram.is_none() && targets.messenger.is_none(),
            "a test server's config names no bot and no page"
        );
        assert!(
            channel_menu_publish(&resources, targets).is_none(),
            "with no target there is nothing to publish"
        );
    }
}
