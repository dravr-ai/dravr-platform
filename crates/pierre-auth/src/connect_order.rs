// ABOUTME: The operator's per-provider connect order for Strava and Garmin: their OAuth first, or the account sign-in first
// ABOUTME: Read from system_settings on every decision, so `pierre-cli settings connect-order` flips it with no deploy
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which way a new Strava or Garmin connection goes first (carnet#1472).
//!
//! Both providers can be connected two ways: their own OAuth, capped in
//! seats, and the account sign-in (the usr/pwd mirror). By default a new
//! connection goes through the OAuth while a seat is free and signs in once
//! the seats are gone ([`ConnectOrder::OAuthFirst`]). The operator can flip a
//! provider to [`ConnectOrder::SignInFirst`]: a new connection then signs
//! in, whatever the seats, and the OAuth stays open to anyone who starts it.
//!
//! The order decides new connections only. An athlete already served by a
//! grant keeps reconnecting through it, and nothing is re-routed or asked to
//! move when the order flips.

use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::backends::OAuthTokenRepository;
use pierre_database::database::system_settings::{
    SETTING_CONNECT_ORDER_GARMIN, SETTING_CONNECT_ORDER_STRAVA,
};
use pierre_database::repositories::SystemSettingsRepository;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::config::oauth::OAuthConfig;
use crate::strava_pool;

/// Which way a new connection of one provider goes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectOrder {
    /// The provider's OAuth while a seat is free, then the account sign-in.
    #[serde(rename = "oauth_first")]
    OAuthFirst,
    /// The account sign-in, whatever the seats; the OAuth stays startable.
    #[serde(rename = "sign_in_first")]
    SignInFirst,
}

impl ConnectOrder {
    /// The stored and wire form: `oauth_first` or `sign_in_first`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OAuthFirst => "oauth_first",
            Self::SignInFirst => "sign_in_first",
        }
    }

    /// Parse the stored or wire form; `None` for anything else.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "oauth_first" => Some(Self::OAuthFirst),
            "sign_in_first" => Some(Self::SignInFirst),
            _ => None,
        }
    }
}

/// The providers whose connect order the operator sets, by their OAuth key.
pub const CONNECT_ORDER_PROVIDERS: [&str; 2] = [oauth_providers::STRAVA, oauth_providers::GARMIN];

/// The `system_settings` key holding `provider`'s order, or `None` for a
/// provider with no order to set.
fn setting_key(provider: &str) -> Option<&'static str> {
    match provider {
        oauth_providers::STRAVA => Some(SETTING_CONNECT_ORDER_STRAVA),
        oauth_providers::GARMIN => Some(SETTING_CONNECT_ORDER_GARMIN),
        _ => None,
    }
}

/// The order a new `provider` connection follows right now.
///
/// Never fails: a row that cannot be read reads as
/// [`ConnectOrder::OAuthFirst`], the order the product had before the
/// setting existed, as everything [`read_connect_order`] maps to it does.
/// For the connect surfaces, which must answer whatever the store does.
pub async fn connect_order(
    settings: &dyn SystemSettingsRepository,
    provider: &str,
) -> ConnectOrder {
    read_connect_order(settings, provider)
        .await
        .unwrap_or_else(|e| {
            warn!(provider, error = %e, "connect order could not be read; using oauth_first");
            ConnectOrder::OAuthFirst
        })
}

/// The order a new `provider` connection follows, or the store's error.
///
/// An absent row and one holding neither form read as
/// [`ConnectOrder::OAuthFirst`], which is what is in force then, and so
/// does any provider outside [`CONNECT_ORDER_PROVIDERS`], which none of the
/// connect surfaces consults. For the operator's read, where a failed read
/// must not be shown as the default.
///
/// # Errors
///
/// The store's error when the row cannot be read.
pub async fn read_connect_order(
    settings: &dyn SystemSettingsRepository,
    provider: &str,
) -> AppResult<ConnectOrder> {
    let Some(key) = setting_key(provider) else {
        return Ok(ConnectOrder::OAuthFirst);
    };
    let Some(setting) = settings.get_setting(key).await? else {
        return Ok(ConnectOrder::OAuthFirst);
    };
    Ok(ConnectOrder::parse(&setting.value).unwrap_or_else(|| {
        warn!(
            setting = key,
            "connect order is neither oauth_first nor sign_in_first; using oauth_first"
        );
        ConnectOrder::OAuthFirst
    }))
}

/// Store `provider`'s order.
///
/// # Errors
///
/// An invalid-input error for a provider outside [`CONNECT_ORDER_PROVIDERS`],
/// before anything is written, and the store's error when the write fails.
pub async fn set_connect_order(
    settings: &dyn SystemSettingsRepository,
    provider: &str,
    order: ConnectOrder,
) -> AppResult<()> {
    let key = setting_key(provider).ok_or_else(|| {
        AppError::invalid_input(format!(
            "no connect order for provider '{provider}'; expected one of: {}",
            CONNECT_ORDER_PROVIDERS.join(", ")
        ))
    })?;
    settings.set_setting(key, order.as_str()).await
}

/// Whether a NEW connection goes through the provider's OAuth.
///
/// Under `order` with `seats_left` free on its client: OAuth first and a seat
/// to take; otherwise it signs in with the account. The one rule the provider
/// card's recommendation and the chat's connect read (carnet#1477).
#[must_use]
pub fn new_connection_through_oauth(order: ConnectOrder, seats_left: u32) -> bool {
    order == ConnectOrder::OAuthFirst && seats_left > 0
}

/// Whether a NEW connection's failed account sign-in falls back to the OAuth.
///
/// The sign-in goes first by the operator's order while a seat is still free.
/// With the pool full there is no OAuth to fall back to.
#[must_use]
pub fn sign_in_falls_back_to_oauth(order: ConnectOrder, seats_left: u32) -> bool {
    order == ConnectOrder::SignInFirst && seats_left > 0
}

/// Strava's connect order and the seats left on its OAuth pool, read once.
///
/// A seat count that fails reads as none left, as the provider card reads
/// it, so nothing is sent into an OAuth that cannot finish.
pub async fn strava_order_and_seats(
    settings: &dyn SystemSettingsRepository,
    tokens: &dyn OAuthTokenRepository,
    server: &OAuthConfig,
) -> (ConnectOrder, u32) {
    let order = connect_order(settings, oauth_providers::STRAVA).await;
    let seats = match strava_pool::strava_seat_summary(tokens, server).await {
        Ok(summary) => summary.left(),
        Err(e) => {
            debug!(error = %e, "Strava seats could not be counted; reading as none left");
            0
        }
    };
    (order, seats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pierre_database::database::system_settings::SystemSetting;

    /// A one-row store, or one whose every read fails: the database fault the
    /// real backends cannot be made to raise on demand. The integration suite
    /// `connect_order_test` drives the real store.
    struct OneRowStore(Result<Option<&'static str>, ()>);

    #[async_trait::async_trait]
    impl SystemSettingsRepository for OneRowStore {
        async fn get_setting(&self, key: &str) -> AppResult<Option<SystemSetting>> {
            self.0
                .map(|value| {
                    value.map(|value| SystemSetting {
                        key: key.to_owned(),
                        value: value.to_owned(),
                        description: None,
                        updated_at: Utc::now(),
                    })
                })
                .map_err(|()| AppError::internal("the store is down"))
        }

        async fn set_setting(&self, _key: &str, _value: &str) -> AppResult<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn a_failed_read_is_an_error_to_the_operator_and_oauth_first_to_a_connection() {
        let down = OneRowStore(Err(()));
        assert!(
            read_connect_order(&down, oauth_providers::STRAVA)
                .await
                .is_err(),
            "bug: a failed read is shown to the operator as an order"
        );
        assert_eq!(
            connect_order(&down, oauth_providers::STRAVA).await,
            ConnectOrder::OAuthFirst,
            "bug: a failed read leaves a connection with no order"
        );
    }

    #[tokio::test]
    async fn the_stored_order_is_read_and_an_absent_or_unknown_row_reads_oauth_first() {
        for (row, expected) in [
            (Some("sign_in_first"), ConnectOrder::SignInFirst),
            (Some("oauth_first"), ConnectOrder::OAuthFirst),
            (Some("sciotte_first"), ConnectOrder::OAuthFirst),
            (None, ConnectOrder::OAuthFirst),
        ] {
            assert_eq!(
                read_connect_order(&OneRowStore(Ok(row)), oauth_providers::GARMIN)
                    .await
                    .ok(),
                Some(expected),
                "bug: row {row:?} reads as another order"
            );
        }
    }

    #[test]
    fn the_order_round_trips_through_its_stored_form_and_refuses_anything_else() {
        for order in [ConnectOrder::OAuthFirst, ConnectOrder::SignInFirst] {
            assert_eq!(
                ConnectOrder::parse(order.as_str()),
                Some(order),
                "bug: {order:?} round trip"
            );
        }
        assert_eq!(
            ConnectOrder::parse(" sign_in_first\n"),
            Some(ConnectOrder::SignInFirst),
            "bug: a stored form with surrounding whitespace is refused"
        );
        assert_eq!(
            ConnectOrder::parse("sciotte_first"),
            None,
            "bug: an unknown form parses"
        );
    }

    #[test]
    fn only_strava_and_garmin_carry_an_order_each_under_its_own_key() {
        let strava = setting_key(oauth_providers::STRAVA);
        let garmin = setting_key(oauth_providers::GARMIN);
        assert!(
            strava.is_some() && garmin.is_some(),
            "bug: an operator-ordered provider has no setting"
        );
        assert_ne!(strava, garmin, "bug: one flip would move both providers");
        assert_eq!(
            setting_key(oauth_providers::WHOOP),
            None,
            "bug: a provider with no sign-in carries an order"
        );
        assert_eq!(
            setting_key(oauth_providers::SCIOTTE_GARMIN),
            None,
            "bug: a mirror key carries an order"
        );
    }

    #[test]
    fn a_new_connection_takes_the_oauth_only_when_it_goes_first_with_a_seat() {
        assert!(
            new_connection_through_oauth(ConnectOrder::OAuthFirst, 3),
            "bug: OAuth first with a seat"
        );
        assert!(
            !new_connection_through_oauth(ConnectOrder::OAuthFirst, 0),
            "bug: a full pool signs in"
        );
        assert!(
            !new_connection_through_oauth(ConnectOrder::SignInFirst, 3),
            "bug: sign-in first signs in"
        );
    }

    #[test]
    fn a_failed_sign_in_falls_back_only_while_the_sign_in_goes_first_with_a_seat() {
        assert!(
            sign_in_falls_back_to_oauth(ConnectOrder::SignInFirst, 3),
            "bug: no fallback with a seat free"
        );
        assert!(
            !sign_in_falls_back_to_oauth(ConnectOrder::SignInFirst, 0),
            "bug: a fallback into a full pool"
        );
        assert!(
            !sign_in_falls_back_to_oauth(ConnectOrder::OAuthFirst, 3),
            "bug: OAuth first never signs in first"
        );
    }
}
