// ABOUTME: Where a channel's linking URL sends the athlete's one-time binding code
// ABOUTME: Resolves each destination from stored credentials and refuses when a config names none
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use dravr_canot::http_client::describe_request_error;
use pierre_config::utils::http_client::shared_client;
use pierre_core::errors::AppError;
use pierre_core::http_client::SharedHttpError;
use pierre_core::models::messaging::ChannelType;
use tracing::info;

/// The Telegram Bot API origin. Every method is `{origin}/bot{token}/{method}`.
///
/// [`build_linking_url`] is the route's only way to a Telegram URL, and it
/// takes no origin: it asks here, so the route cannot hand it another one.
const TELEGRAM_API: &str = "https://api.telegram.org";

/// Cache of bot token -> username, so `getMe` is called once per token rather
/// than once per link request. A bot's username changes only when an operator
/// renames it in `BotFather`, and a stale entry would send codes to a handle that
/// no longer resolves, so the process lifetime is the right bound: a redeploy
/// re-reads it.
static TELEGRAM_BOT_USERNAMES: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The Telegram bot a link code must be sent to.
///
/// Asked of Telegram, not configured. `getMe` returns the username belonging to
/// the bot token we already store, which makes the answer correct by
/// construction — there is no value for an operator to set, mistype, or leave
/// stale, and no second place for it to drift from.
///
/// That matters more here than it usually would. This URL is where the athlete
/// sends a one-time code that binds whoever sends it to their account, so a
/// wrong handle is not a broken link, it is a credential disclosure. The
/// previous implementation defaulted to a hardcoded `PierreBot` — a real bot
/// belonging to a stranger — and every link went there, because the config
/// write path never persisted a `bot_username` for the default to fall back
/// from. An environment variable would have fixed that instance while leaving
/// the same shape in place: a human-supplied name that nothing verifies.
///
/// A missing or rejected token is an error. Guessing is never correct.
///
/// `telegram_api` is the Bot API origin `getMe` is asked at: [`TELEGRAM_API`]
/// in the server, the same way `exchange_code_for_identity` is handed the
/// provider endpoints it calls. Only the Telegram lookup takes it; the other
/// channels build their URL from the config alone.
async fn telegram_bot_username(
    config: &serde_json::Value,
    telegram_api: &str,
) -> Result<String, AppError> {
    let token = config
        .get("bot_token")
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AppError::internal(
                "Telegram channel has no bot_token, so the bot's username cannot be \
                 resolved and no linking URL can be built. Configure the channel \
                 before issuing link codes.",
            )
        })?;

    if let Some(cached) = TELEGRAM_BOT_USERNAMES
        .read()
        .ok()
        .and_then(|m| m.get(token).cloned())
    {
        return Ok(cached);
    }

    // The token sits in the request path and a `reqwest::Error` displays that
    // URL, while `AppError::internal` details reach Cloud Logging. Transport
    // errors go through canot's `describe_request_error`, which strips the URL;
    // a middleware error cannot be stripped, so the token is masked out of it.
    let url = format!("{telegram_api}/bot{token}/getMe");
    let response = shared_client().get(&url).send().await.map_err(|e| {
        let detail = match e {
            SharedHttpError::Reqwest(e) => describe_request_error(e),
            SharedHttpError::Middleware(e) => format!("{e:#}").replace(token, "***"),
        };
        AppError::internal(format!("Telegram getMe request failed: {detail}"))
    })?;

    let body: serde_json::Value = response.json().await.map_err(|e| {
        AppError::internal(format!(
            "Telegram getMe returned no JSON: {}",
            describe_request_error(e)
        ))
    })?;

    let username = username_in_get_me(&body)?;

    if let Ok(mut cache) = TELEGRAM_BOT_USERNAMES.write() {
        cache.insert(token.to_owned(), username.clone());
    }
    info!(bot_username = %username, "resolved the Telegram bot username via getMe");
    Ok(username)
}

/// The bot's username as Telegram's `getMe` answer states it.
///
/// An answer without one is an error, never a default: a rejected token, a
/// deleted bot and a changed response shape all land here, and each means the
/// platform does not know which bot a linking code would be sent to.
fn username_in_get_me(body: &serde_json::Value) -> Result<String, AppError> {
    body.get("result")
        .and_then(|r| r.get("username"))
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            // Deliberately does not log the body: a rejected token comes back
            // with a description that can echo the token itself.
            AppError::internal(
                "Telegram getMe did not return a username — the stored bot_token is \
                 rejected or the bot was deleted. Refusing to build a linking URL \
                 without knowing which bot it points at.",
            )
        })
}

/// The config keys `build_linking_url` needs before it can build a URL for this
/// channel at all.
///
/// This exists so the channel picker can withhold a channel whose link cannot
/// complete, instead of advertising a button that fails the moment the athlete
/// taps it. `build_linking_url` below reads exactly these keys and errors when
/// one is absent — the two must agree, so they sit next to each other and any
/// new channel has to answer both in the same edit.
///
/// Presence only. Nothing here reads, logs or returns a credential's value.
const fn required_credential_keys(channel_type: ChannelType) -> &'static [&'static str] {
    match channel_type {
        // The bot the code is sent to; resolved to a username via `getMe`.
        ChannelType::Telegram => &["bot_token"],
        // The number the pre-filled message is addressed to.
        ChannelType::WhatsApp => &["phone_number"],
        // The page the `m.me` link opens.
        ChannelType::Messenger => &["account_id"],
        // The authorize URL needs the client id, and the callback that follows
        // needs the secret to exchange the code, so a link completes only with
        // both.
        ChannelType::Slack | ChannelType::Discord => &["api_key", "api_secret"],
    }
}

/// Whether a channel config carries every credential its linking flow needs.
///
/// Reads only for presence — never logs or returns the values.
pub fn can_complete_a_link(channel_type: ChannelType, config: &serde_json::Value) -> bool {
    required_credential_keys(channel_type).iter().all(|key| {
        config
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty())
    })
}

/// The Telegram deep link that sends `code` to the bot the stored token
/// belongs to, whose username is asked of the Bot API at `telegram_api`.
async fn telegram_linking_url(
    code: &str,
    config: &serde_json::Value,
    telegram_api: &str,
) -> Result<String, AppError> {
    // No fallback bot, deliberately. This used to default to
    // "PierreBot" when the channel config carried no `bot_username` —
    // and it always did, because the config write path never persists
    // that key (see `config.rs`, which extracts api_key / api_secret /
    // webhook_secret / verify_token / account_id / phone_number /
    // bot_token and nothing else). So EVERY link pointed at
    // https://t.me/PierreBot, which is a real bot belonging to a
    // stranger.
    //
    // That is worse than a dead link. `detect_linking_code` +
    // `execute_link_code` bind whoever sends the code to the requesting
    // athlete's account inside the TTL, so pressing Start on that
    // third-party bot hands an account-binding credential off-platform.
    //
    // Guessing a bot name is therefore never acceptable: an absent
    // username must fail loudly rather than produce a plausible URL
    // aimed at someone else's bot.
    let bot_username = telegram_bot_username(config, telegram_api).await?;
    Ok(format!("https://t.me/{bot_username}?start={code}"))
}

/// Build the linking URL based on channel type and method.
///
/// Reads the keys [`required_credential_keys`] names for this channel and errors
/// when one is absent, rather than guessing a target. A Telegram bot's username
/// is asked of the real Bot API, [`TELEGRAM_API`]; see [`telegram_linking_url`].
pub(super) async fn build_linking_url(
    channel_type: ChannelType,
    code: &str,
    config: &serde_json::Value,
    base_url: &str,
) -> Result<String, AppError> {
    match channel_type {
        ChannelType::Telegram => telegram_linking_url(code, config, TELEGRAM_API).await,
        ChannelType::WhatsApp => {
            // Same rule as Telegram above, for the same reason. An empty number
            // yields `https://wa.me/?text=LINK+CODE`, which is not a dead link:
            // WhatsApp opens the contact picker with the athlete's one-time
            // binding code already typed, and whoever they pick receives it.
            // Milder than aiming at a stranger's bot only because it takes a
            // tap — the failure is identical in kind, so it fails identically.
            let phone = config
                .get("phone_number")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    AppError::internal(
                        "WhatsApp channel has no phone_number, so there is no recipient \
                         for the link code. Refusing to build a URL that would open a \
                         contact picker with the athlete's binding code pre-filled.",
                    )
                })?;
            let message_text = format!("LINK {code}");
            let encoded_message = urlencoding::encode(&message_text);
            Ok(format!("https://wa.me/{phone}?text={encoded_message}"))
        }
        ChannelType::Messenger => {
            // Same rule as Telegram and WhatsApp above: no guessed target. The
            // page id identifies which Messenger page the link opens, and an
            // absent one previously produced `.../link/callback/messenger?state=`
            // — our own endpoint, which rejects the request for the
            // `channel_user_id` nothing supplies. That is the 400 every Messenger
            // link attempt hit.
            //
            // `ref` is Messenger's own deep-link parameter and comes back on the
            // webhook (dravr-canot >= 0.4.20 parses it from both the bare
            // `referral` and the `postback.referral` shape), which is what makes
            // it the equivalent of Telegram's `?start=`.
            let page_id = config
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    AppError::internal(
                        "Messenger channel has no account_id, so there is no page for the \
                         link to open. Refusing to build a URL that cannot complete.",
                    )
                })?;
            Ok(format!("https://m.me/{page_id}?ref={code}"))
        }
        // Genuine OAuth channels. These used to return our OWN callback with only
        // a `state` param — an endpoint that rejects the request for the
        // `channel_user_id` nothing supplies, so every attempt 400'd. The user
        // has to be sent to the provider first; the provider is what knows who
        // they are, which is the whole point of the round trip.
        //
        // `api_key` carries the OAuth client id. The picker already refuses to
        // advertise a channel whose credentials are absent, and this refuses to
        // build a URL for one anyway — the same refuse-to-guess rule the
        // deep-link channels follow.
        ChannelType::Slack | ChannelType::Discord => {
            let client_id = config
                .get("api_key")
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    AppError::internal(format!(
                        "{channel_type} channel has no OAuth client id, so the authorize URL \
                         cannot identify this app. Refusing to build a link that cannot complete."
                    ))
                })?;

            let redirect_uri = format!("{base_url}/api/messaging/link/callback/{channel_type}");
            // Identity only. Linking needs to learn who the person is and
            // nothing else; a broader scope would ask for consent we have no
            // use for, which is both a worse prompt and more to leak.
            let (authorize, scope) = match channel_type {
                ChannelType::Slack => ("https://slack.com/openid/connect/authorize", "openid"),
                _ => ("https://discord.com/oauth2/authorize", "identify"),
            };

            Ok(format!(
                "{authorize}?response_type=code&client_id={}&scope={}&redirect_uri={}&state={}",
                urlencoding::encode(client_id),
                urlencoding::encode(scope),
                urlencoding::encode(&redirect_uri),
                urlencoding::encode(code),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    //! Where a linking URL sends the athlete's one-time binding code.
    //!
    //! `build_linking_url` used to default the Telegram handle to `PierreBot` —
    //! a real bot belonging to a stranger — and every link went there, because
    //! no config ever carried the `bot_username` the default fell back from.
    //! `detect_linking_code` binds whoever sends a valid code to the requesting
    //! athlete's account, so a guessed destination is a credential disclosure.
    //!
    //! Each test builds a URL the way `init_channel_link` does and reads where
    //! it points. `linking_destination_test` drives the same rule through the
    //! route, where a refusal must also leave no redeemable code behind.
    //!
    //! An environment variable was the first fix attempted for the Telegram
    //! handle. It keeps the shape of the hardcoded default — a name a person
    //! types that nothing verifies — so the last tests here set one and walk
    //! every way out of `telegram_bot_username` past the token check, one test
    //! each:
    //!
    //! - a cached answer for the token;
    //! - a `getMe` answer naming the bot;
    //! - a `getMe` that cannot be reached (the request fails);
    //! - a `getMe` that answers a body that is not JSON;
    //! - a `getMe` answer naming no bot (a rejected token, a nameless bot).
    //!
    //! The way out before the token check, a config with no `bot_token`, is
    //! covered here for every channel and by the route test with the variable
    //! set.

    use std::env;

    use axum::extract::Path;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::{json, Value};
    use serial_test::serial;
    use tokio::net::TcpListener;

    use super::{
        build_linking_url, telegram_linking_url, username_in_get_me, ChannelType,
        TELEGRAM_BOT_USERNAMES,
    };

    const CODE: &str = "Code123";
    const BASE_URL: &str = "https://api.test.dravr.ai";

    /// A Telegram origin for tests that must settle without asking `getMe`.
    /// Nothing listens on port 1, so reaching it is an error, not an answer.
    const UNREACHED: &str = "http://127.0.0.1:1";

    /// The environment variable an earlier draft of the fix read the Telegram
    /// handle from. Named here to keep it out, not to support it.
    const REJECTED_ENV_VAR: &str = "PIERRE_TELEGRAM_BOT_USERNAME";

    /// The name typed into [`REJECTED_ENV_VAR`]. Never a destination.
    const TYPED: &str = "TypedIntoTheEnvironmentBot";

    /// Sets [`REJECTED_ENV_VAR`] for one test and puts back whatever was there
    /// before, on a panic too. The variable is process-global, so every test
    /// holding one is `#[serial]`.
    struct TypedBotName {
        previous: Option<String>,
    }

    impl TypedBotName {
        fn set() -> Self {
            let previous = env::var(REJECTED_ENV_VAR).ok();
            env::set_var(REJECTED_ENV_VAR, TYPED);
            assert_eq!(
                env::var(REJECTED_ENV_VAR).as_deref(),
                Ok(TYPED),
                "premise: the variable is set while the URL is built"
            );
            Self { previous }
        }
    }

    impl Drop for TypedBotName {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => env::set_var(REJECTED_ENV_VAR, value),
                None => env::remove_var(REJECTED_ENV_VAR),
            }
        }
    }

    /// A Bot API stand-in answering `getMe` for `token` with `answer`, on an
    /// ephemeral port. Returns its origin.
    async fn telegram_answering(token: &str, answer: Value) -> String {
        let expected = format!("bot{token}");
        let app = Router::new().route(
            "/{bot}/getMe",
            get(move |Path(bot): Path<String>| async move {
                if bot == expected {
                    Json(answer).into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    /// A Bot API stand-in answering `getMe` for `token` with a 200 whose body
    /// is not JSON, as a proxy or captive portal in front of Telegram would.
    async fn telegram_answering_text(token: &str, body: &'static str) -> String {
        let expected = format!("bot{token}");
        let app = Router::new().route(
            "/{bot}/getMe",
            get(move |Path(bot): Path<String>| async move {
                if bot == expected {
                    body.into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    /// An origin nothing listens on: the port was bound, read, and released.
    async fn telegram_unreachable() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{addr}")
    }

    /// A refusal names no destination at all: not the typed name, not a URL.
    fn assert_refused(result: &Result<String, super::AppError>, when: &str) {
        let Err(error) = result else {
            panic!("{when}: the link must be refused, got {result:?}");
        };
        let said = format!("{error} {error:?}");
        assert!(
            !said.contains(TYPED) && !said.contains("t.me/"),
            "{when}: {REJECTED_ENV_VAR} must not become a destination: {said}"
        );
    }

    /// Stand in for a `getMe` round trip that already happened for `token`.
    fn telegram_answered(token: &str, username: &str) {
        TELEGRAM_BOT_USERNAMES
            .write()
            .unwrap()
            .insert(token.to_owned(), username.to_owned());
    }

    #[test]
    fn get_me_names_the_bot_the_token_belongs_to() {
        let answer = json!({
            "ok": true,
            "result": { "id": 42, "is_bot": true, "username": "dravr_coach_bot" }
        });
        assert_eq!(username_in_get_me(&answer).unwrap(), "dravr_coach_bot");
    }

    #[test]
    fn a_get_me_answer_without_a_username_is_an_error_not_a_default() {
        for answer in [
            json!({ "ok": false, "error_code": 401, "description": "Unauthorized" }),
            json!({ "ok": true, "result": { "id": 42 } }),
            json!({ "ok": true, "result": { "username": "" } }),
            json!({}),
        ] {
            assert!(
                username_in_get_me(&answer).is_err(),
                "no username means no known bot: {answer}"
            );
        }
    }

    /// Through the function the route calls, which takes no origin: the
    /// token's resolved bot is the destination.
    #[tokio::test]
    async fn a_telegram_link_points_at_the_bot_its_token_resolved_to() {
        telegram_answered("111:resolved-token", "dravr_coach_bot");

        let url = build_linking_url(
            ChannelType::Telegram,
            CODE,
            &json!({ "bot_token": "111:resolved-token" }),
            BASE_URL,
        )
        .await
        .unwrap();

        assert_eq!(url, "https://t.me/dravr_coach_bot?start=Code123");
    }

    /// A name a person typed is not a destination: only the stored token's own
    /// answer is. A config carrying `bot_username` and no token has no bot.
    #[tokio::test]
    async fn a_configured_bot_name_is_never_a_destination() {
        let result = telegram_linking_url(
            CODE,
            &json!({ "bot_username": "SomebodyElsesBot" }),
            UNREACHED,
        )
        .await;

        assert!(
            result.is_err(),
            "a typed bot name must not become a linking URL: {result:?}"
        );
    }

    /// The token decides, even when a typed name sits beside it.
    #[tokio::test]
    async fn the_token_outranks_a_configured_bot_name() {
        telegram_answered("222:resolved-token", "dravr_coach_bot");

        let url = telegram_linking_url(
            CODE,
            &json!({ "bot_token": "222:resolved-token", "bot_username": "SomebodyElsesBot" }),
            UNREACHED,
        )
        .await
        .unwrap();

        assert_eq!(url, "https://t.me/dravr_coach_bot?start=Code123");
    }

    /// Every channel either names the destination its config resolves to, or
    /// refuses. None may fall back to a default.
    #[tokio::test]
    async fn every_channel_refuses_a_config_that_names_no_destination() {
        for channel in [
            ChannelType::Telegram,
            ChannelType::WhatsApp,
            ChannelType::Messenger,
            ChannelType::Slack,
            ChannelType::Discord,
        ] {
            for config in [
                json!({}),
                json!({
                    "bot_token": "", "phone_number": "", "account_id": "", "api_key": ""
                }),
            ] {
                let result = build_linking_url(channel, CODE, &config, BASE_URL).await;
                assert!(
                    result.is_err(),
                    "{channel} built a URL from a config naming no destination: {result:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn each_channel_points_at_the_destination_its_config_names() {
        for (channel, config, expected) in [
            (
                ChannelType::WhatsApp,
                json!({ "phone_number": "15551234567" }),
                "https://wa.me/15551234567?text=LINK%20Code123".to_owned(),
            ),
            (
                ChannelType::Messenger,
                json!({ "account_id": "page-1" }),
                "https://m.me/page-1?ref=Code123".to_owned(),
            ),
            (
                ChannelType::Slack,
                json!({ "api_key": "slack-id" }),
                "https://slack.com/openid/connect/authorize?response_type=code\
                 &client_id=slack-id&scope=openid\
                 &redirect_uri=https%3A%2F%2Fapi.test.dravr.ai%2Fapi%2Fmessaging%2Flink%2Fcallback%2Fslack\
                 &state=Code123"
                    .to_owned(),
            ),
            (
                ChannelType::Discord,
                json!({ "api_key": "discord-id" }),
                "https://discord.com/oauth2/authorize?response_type=code\
                 &client_id=discord-id&scope=identify\
                 &redirect_uri=https%3A%2F%2Fapi.test.dravr.ai%2Fapi%2Fmessaging%2Flink%2Fcallback%2Fdiscord\
                 &state=Code123"
                    .to_owned(),
            ),
        ] {
            let url = build_linking_url(channel, CODE, &config, BASE_URL)
                .await
                .unwrap();
            assert_eq!(url, expected, "{channel}");
        }
    }

    /// Past the token check, on the cached answer: the bot the token resolved
    /// to is the destination, whatever the environment names.
    #[tokio::test]
    #[serial]
    async fn a_cached_bot_outranks_a_name_typed_into_the_environment() {
        let typed_bot_name = TypedBotName::set();
        telegram_answered("333:cached-token", "dravr_coach_bot");

        let url =
            telegram_linking_url(CODE, &json!({ "bot_token": "333:cached-token" }), UNREACHED)
                .await
                .unwrap();

        assert_eq!(url, "https://t.me/dravr_coach_bot?start=Code123");
        drop(typed_bot_name);
    }

    /// The same on a first resolution: `getMe` is asked, and its answer is the
    /// destination. This is also what shows the two refusals below are reached
    /// through a `getMe` call rather than short of one.
    #[tokio::test]
    #[serial]
    async fn the_get_me_answer_outranks_a_name_typed_into_the_environment() {
        let typed_bot_name = TypedBotName::set();
        let token = "444:answered-token";
        let telegram = telegram_answering(
            token,
            json!({ "ok": true, "result": { "id": 42, "username": "dravr_coach_bot" } }),
        )
        .await;

        let url = telegram_linking_url(CODE, &json!({ "bot_token": token }), &telegram)
            .await
            .unwrap();

        assert_eq!(url, "https://t.me/dravr_coach_bot?start=Code123");
        drop(typed_bot_name);
    }

    /// Telegram cannot be reached, so nobody knows which bot the token belongs
    /// to. That is an error; the typed name does not fill the gap.
    #[tokio::test]
    #[serial]
    async fn an_unreachable_get_me_is_an_error_not_the_typed_name() {
        let typed_bot_name = TypedBotName::set();
        let telegram = telegram_unreachable().await;

        let result = telegram_linking_url(
            CODE,
            &json!({ "bot_token": "555:unreachable-token" }),
            &telegram,
        )
        .await;

        assert_refused(&result, "getMe transport failure");
        drop(typed_bot_name);
    }

    /// Telegram answers and names no bot — a rejected token, or a bot with no
    /// username in the answer. An error; the typed name does not fill the gap.
    #[tokio::test]
    #[serial]
    async fn a_get_me_answer_naming_no_bot_is_an_error_not_the_typed_name() {
        let typed_bot_name = TypedBotName::set();

        for (token, answer) in [
            (
                "666:rejected-token",
                json!({ "ok": false, "error_code": 401, "description": "Unauthorized" }),
            ),
            (
                "777:nameless-token",
                json!({ "ok": true, "result": { "id": 42, "is_bot": true } }),
            ),
        ] {
            let telegram = telegram_answering(token, answer.clone()).await;

            let result =
                telegram_linking_url(CODE, &json!({ "bot_token": token }), &telegram).await;

            assert_refused(&result, &format!("getMe answered {answer}"));
        }
        drop(typed_bot_name);
    }

    /// Telegram answers 200 with a body that is not JSON, so nothing names the
    /// bot. An error; the typed name does not fill the gap.
    #[tokio::test]
    #[serial]
    async fn a_get_me_answer_that_is_not_json_is_an_error_not_the_typed_name() {
        let typed_bot_name = TypedBotName::set();
        let token = "888:html-token";
        let telegram =
            telegram_answering_text(token, "<html><body>Bad Gateway</body></html>").await;

        let result = telegram_linking_url(CODE, &json!({ "bot_token": token }), &telegram).await;

        assert_refused(&result, "getMe answered a body that is not JSON");
        drop(typed_bot_name);
    }
}
