// ABOUTME: `pierre-cli settings connect-order` — show or flip which way a new Strava or Garmin connection goes first
// ABOUTME: Thin HTTP wrapper over /admin/settings/connect-order with the cached device-login token (super-admin)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Remote management of the per-provider connect order (carnet#1472).
//!
//! `oauth` first (the default): a new connection goes through the provider's
//! OAuth while a seat is free, then the account sign-in. `sign-in` first: the
//! account sign-in whatever the seats, the OAuth staying open to whoever
//! starts it. Existing connections are never re-routed.

use clap::{Subcommand, ValueEnum};
use pierre_auth::connect_order::{ConnectOrder, CONNECT_ORDER_PROVIDERS};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::AppResult;
use serde_json::{json, Value};

use super::settings::{client, display_value};

const CONNECT_ORDER_PATH: &str = "/admin/settings/connect-order";

/// The providers that carry a connect order.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum OrderProvider {
    /// Strava
    Strava,
    /// Garmin
    Garmin,
}

impl OrderProvider {
    const fn wire(self) -> &'static str {
        match self {
            Self::Strava => oauth_providers::STRAVA,
            Self::Garmin => oauth_providers::GARMIN,
        }
    }
}

/// Which way a new connection goes first.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum FirstWay {
    /// The provider's OAuth while a seat is free, then the account sign-in
    Oauth,
    /// The account sign-in, whatever the seats
    SignIn,
}

impl FirstWay {
    const fn order(self) -> ConnectOrder {
        match self {
            Self::Oauth => ConnectOrder::OAuthFirst,
            Self::SignIn => ConnectOrder::SignInFirst,
        }
    }
}

/// Verbs for `settings connect-order`.
#[non_exhaustive]
#[derive(Subcommand)]
pub(crate) enum ConnectOrderAction {
    /// Show each provider's connect order
    Show,
    /// Set one provider's connect order, for new connections
    Set {
        /// The provider to flip
        #[arg(long, value_enum)]
        provider: OrderProvider,
        /// Which way a new connection goes first
        #[arg(long, value_enum)]
        first: FirstWay,
    },
}

/// Dispatch a `settings connect-order` verb.
///
/// # Errors
/// Returns an error if not logged in, or the server refuses the request (a
/// token without super-admin is refused).
pub(crate) async fn dispatch(action: ConnectOrderAction) -> AppResult<()> {
    match action {
        ConnectOrderAction::Show => {
            let response = client()?.get_json(CONNECT_ORDER_PATH).await?;
            print_orders(&response);
        }
        ConnectOrderAction::Set { provider, first } => {
            let body = set_body(provider, first);
            let response = client()?.put_json(CONNECT_ORDER_PATH, &body).await?;
            println!(
                "  {} now connects {} first, for new connections.",
                provider.wire(),
                first_label(first.order().as_str())
            );
            print_orders(&response);
        }
    }
    Ok(())
}

/// The PUT body for one provider's order.
fn set_body(provider: OrderProvider, first: FirstWay) -> Value {
    json!({ "provider": provider.wire(), "order": first.order().as_str() })
}

/// The operator's words for a stored order; a form this build does not know
/// is shown as it came.
fn first_label(order: &str) -> &str {
    match ConnectOrder::parse(order) {
        Some(ConnectOrder::OAuthFirst) => "OAuth",
        Some(ConnectOrder::SignInFirst) => "sign-in",
        None => order,
    }
}

fn print_orders(response: &Value) {
    println!("  Connect order (new connections only)");
    for provider in CONNECT_ORDER_PROVIDERS {
        let order = display_value(response.pointer(&format!("/data/orders/{provider}")));
        println!("  {provider:<8}  {} first  ({order})", first_label(&order));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The literals are wire values: the PUT body `/admin/settings/connect-order` parses.
    #[test]
    fn the_flags_send_the_wire_forms_the_server_parses() {
        assert_eq!(
            set_body(OrderProvider::Strava, FirstWay::SignIn),
            json!({ "provider": "strava", "order": "sign_in_first" }),
            "bug: --provider strava --first sign-in sends another body"
        );
        assert_eq!(
            set_body(OrderProvider::Garmin, FirstWay::Oauth),
            json!({ "provider": "garmin", "order": "oauth_first" }),
            "bug: --provider garmin --first oauth sends another body"
        );
    }

    #[test]
    fn a_stored_order_reads_as_the_way_it_puts_first() {
        assert_eq!(
            first_label("oauth_first"),
            "OAuth",
            "bug: the OAuth-first order is mislabelled"
        );
        assert_eq!(
            first_label("sign_in_first"),
            "sign-in",
            "bug: the sign-in-first order is mislabelled"
        );
        assert_eq!(first_label("?"), "?", "bug: an unknown form is hidden");
    }
}
