// ABOUTME: What an external caller (MCP, A2A) reads when the athlete has no provider connected
// ABOUTME: The providers the deployment offers with the athlete's state and whether each is readable on this transport
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The no-provider answer for a caller outside Dravr's own surfaces.
//!
//! Dravr's web, mobile and messaging chat tell the model to send the athlete
//! to the app, and their pipeline keys on the refusal's exact shape
//! ([`no_provider_refusal`](super::provider_helpers::no_provider_refusal)).
//! An MCP client or an A2A agent is a different reader: it writes its own
//! reply to the athlete, a provider that is connected in the app may keep its
//! data off this transport altogether, and "connect Strava" would promise
//! what the transport then withholds.
//!
//! So the external answer names no provider of its own. It carries the list
//! `get_connection_status` gives (every provider the deployment offers, with
//! the athlete's state on each) and, per provider, whether its data can be
//! read on the transport the call arrived by, computed from the registry's
//! declared terms. It is an ordinary result: having nothing connected is the
//! athlete's state, not a failure of the call.

use std::collections::HashMap;

use pierre_core::ai_policy::ProviderTerms;
use pierre_core::models::TenantId;
use pierre_core::transport::Transport;
use serde_json::{json, Map, Value};
use tracing::warn;
use uuid::Uuid;

use crate::protocol::provider_states::{offered_providers, read_status_by_provider};
use crate::protocol::types::{UniversalResponse, META_NO_PROVIDER_CONNECTED};
use crate::runtime::ToolRuntime;

/// One provider the deployment offers, as an external caller reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOffer {
    /// The name `connect_provider` takes.
    pub name: &'static str,
    /// The athlete's state on it, in words (`disconnected`, `connected`, ...).
    pub status: &'static str,
    /// Whether its data can be read over the transport the call arrived by.
    pub readable_here: bool,
}

/// Whether `provider`'s data may be served over `transport`, by the terms the
/// registry holds for it.
///
/// A provider the terms do not know has no restriction, as everywhere else
/// those terms are read.
#[must_use]
pub(crate) fn readable_over(
    terms: &dyn ProviderTerms,
    provider: &str,
    transport: Transport,
) -> bool {
    transport.is_first_party()
        || terms
            .transport_policy(provider)
            .is_none_or(|policy| !policy.is_first_party_only())
}

/// The offers for `user_id`, in provider-name order.
///
/// Empty when the list cannot be read (no tenant, or a failed lookup): the
/// answer then names no provider rather than inventing one.
pub(crate) async fn provider_offers(
    runtime: &dyn ToolRuntime,
    user_id: Uuid,
    tenant: Option<Uuid>,
    transport: Transport,
) -> Vec<ProviderOffer> {
    let Some(tenant) = tenant.map(TenantId::from_uuid) else {
        return Vec::new();
    };
    let repos = runtime.repos();
    let registry = runtime.provider_registry();
    let statuses = match read_status_by_provider(repos, user_id, tenant).await {
        Ok(statuses) => statuses,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "provider list unreadable; the answer names none");
            return Vec::new();
        }
    };
    offered_providers(repos, registry, user_id, tenant, &statuses)
        .await
        .into_iter()
        .map(|(name, state)| ProviderOffer {
            name,
            status: state.status,
            readable_here: readable_over(registry.as_ref(), name, transport),
        })
        .collect()
}

/// The offers as lines a model can read back: `- name (disconnected)`, with
/// a note on the ones whose data stays in the Dravr app.
#[must_use]
pub fn describe_offers(offers: &[ProviderOffer]) -> String {
    offers
        .iter()
        .map(|offer| {
            let status = offer.status.replace('_', " ");
            if offer.readable_here {
                format!("- {} ({status})", offer.name)
            } else {
                format!(
                    "- {} ({status}): its data is readable only in the Dravr app",
                    offer.name
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The words every external caller is told about a provider whose data does
/// not leave the Dravr app.
pub(crate) const APP_ONLY_NOTE: &str = "Its data is readable only in the Dravr app: connecting it \
     there does not make that data available over this interface.";

/// Where an external caller's athlete goes to connect a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectRoute {
    /// The caller's grant covers `connect_provider`.
    Tool,
    /// It does not, so the tool is not listed to it and the athlete connects
    /// in the app: at this address, when the deployment names one.
    App(Option<String>),
}

/// The web app's page for connecting providers, under `frontend_url`.
#[must_use]
pub(crate) fn connections_page(frontend_url: &str) -> String {
    format!(
        "{}/#settings/connections",
        frontend_url.trim_end_matches('/')
    )
}

/// `schema`, the output a provider-requiring tool declares, widened to also
/// accept [`external_no_provider_response`]'s result.
///
/// That result is an ordinary one, and a client validates an ordinary result
/// against the declared schema and fails the call on a mismatch, which is the
/// failure the answer exists to avoid. Keywords a schema's references resolve
/// against stay at the root, because `#/$defs/...` is read from there.
#[must_use]
pub fn accept_no_provider_answer(schema: Value) -> Value {
    let Value::Object(mut declared) = schema else {
        return schema;
    };
    let mut widened = Map::new();
    for keyword in ["$schema", "$defs", "definitions", "title", "description"] {
        if let Some(value) = declared.remove(keyword) {
            widened.insert(keyword.to_owned(), value);
        }
    }
    widened.insert("type".to_owned(), Value::String("object".to_owned()));
    widened.insert(
        "anyOf".to_owned(),
        json!([
            declared,
            {
                "type": "object",
                "required": ["connected", "message", "providers"],
                "properties": {
                    "connected": { "const": false },
                    "message": { "type": "string" },
                    "connect_url": { "type": "string" },
                    "providers": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "required": ["name", "status", "readable_here"],
                            "properties": {
                                "name": { "type": "string" },
                                "status": { "type": "string" },
                                "readable_here": { "type": "boolean" },
                                "note": { "type": "string" }
                            }
                        }
                    }
                }
            }
        ]),
    );
    Value::Object(widened)
}

/// The ordinary result an external caller gets when a tool needs athlete data
/// and no provider is connected.
///
/// Keeps [`META_NO_PROVIDER_CONNECTED`] in its metadata, which never reaches
/// the wire, so the in-process detector still recognises the situation.
#[must_use]
pub(crate) fn external_no_provider_response(
    offers: &[ProviderOffer],
    connect: &ConnectRoute,
) -> UniversalResponse {
    let listing = if offers.is_empty() {
        "get_connection_status lists the providers this Dravr offers and the athlete's \
         state on each."
    } else {
        "The providers this Dravr offers are in `providers`, each with the athlete's state on \
         it and `readable_here`: whether that provider's data can be read over this interface. \
         Where `readable_here` is false its data is readable only in the Dravr app, and stays \
         there even once connected."
    };
    let how_to_connect = match connect {
        ConnectRoute::Tool => {
            "connect_provider starts a connection for the provider the athlete names.".to_owned()
        }
        ConnectRoute::App(Some(url)) => format!("The athlete connects a provider at {url}."),
        ConnectRoute::App(None) => "The athlete connects a provider in the Dravr app.".to_owned(),
    };
    let message = format!(
        "Nothing is connected for this athlete yet, so there is no activity or health data \
         to read. This is the athlete's state, not an error. {listing} Which provider to \
         connect is the athlete's choice. {how_to_connect} A question that does not need their \
         data can still be answered from general knowledge."
    );
    let providers: Vec<Value> = offers
        .iter()
        .map(|offer| {
            let mut entry = json!({
                "name": offer.name,
                "status": offer.status,
                "readable_here": offer.readable_here,
            });
            if !offer.readable_here {
                entry["note"] = Value::String(APP_ONLY_NOTE.to_owned());
            }
            entry
        })
        .collect();
    let mut metadata: HashMap<String, Value> = HashMap::new();
    metadata.insert(META_NO_PROVIDER_CONNECTED.to_owned(), Value::Bool(true));
    let mut result = json!({
        "connected": false,
        "message": message,
        "providers": providers,
    });
    if let ConnectRoute::App(Some(url)) = connect {
        result["connect_url"] = Value::String(url.clone());
    }
    UniversalResponse {
        success: true,
        result: Some(result),
        error: None,
        metadata: Some(metadata),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::types::is_no_provider_refusal;
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::transport::TransportPolicy;

    struct Terms;

    impl ProviderTerms for Terms {
        fn ai_policy(&self, _: &str) -> Option<&'static SourcePolicy> {
            None
        }
        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            match provider {
                "closed" => Some(TransportPolicy::FirstPartyOnly),
                "open" => Some(TransportPolicy::AnyTransport),
                _ => None,
            }
        }
    }

    fn offers() -> Vec<ProviderOffer> {
        vec![
            ProviderOffer {
                name: "closed",
                status: "disconnected",
                readable_here: false,
            },
            ProviderOffer {
                name: "open",
                status: "needs_reauth",
                readable_here: true,
            },
        ]
    }

    #[test]
    fn a_first_party_only_provider_is_unreadable_on_every_external_transport_only() {
        for external in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            assert!(!readable_over(&Terms, "closed", external));
            assert!(readable_over(&Terms, "open", external));
            assert!(readable_over(&Terms, "unknown", external));
        }
        for own in [
            Transport::WebApp,
            Transport::MobileApp,
            Transport::Messaging,
        ] {
            assert!(readable_over(&Terms, "closed", own));
        }
    }

    #[test]
    fn the_answer_is_an_ordinary_result_listing_every_offer() {
        let response = external_no_provider_response(&offers(), &ConnectRoute::Tool);
        assert!(response.success, "bug: nothing connected is not a failure");
        assert!(response.error.is_none());
        let result = response.result.expect("a result");
        let providers = result["providers"].as_array().expect("providers");
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0]["readable_here"], false);
        assert_eq!(providers[1]["status"], "needs_reauth");
        assert_eq!(providers[0]["note"], APP_ONLY_NOTE);
        assert!(providers[1].get("note").is_none());
        let message = result["message"].as_str().expect("message");
        assert!(message.contains("`readable_here`"));
        assert!(is_no_provider_refusal(&external_no_provider_response(
            &offers(),
            &ConnectRoute::Tool
        )));
    }

    #[test]
    fn the_status_tool_lines_name_each_offer_and_where_its_data_stays() {
        assert_eq!(
            describe_offers(&offers()),
            "- closed (disconnected): its data is readable only in the Dravr app\n\
             - open (needs reauth)"
        );
    }

    #[test]
    fn an_unreadable_list_names_no_provider() {
        let response = external_no_provider_response(&[], &ConnectRoute::Tool);
        let result = response.result.expect("a result");
        assert_eq!(result["providers"].as_array().map(Vec::len), Some(0));
        assert!(result["message"]
            .as_str()
            .is_some_and(|m| m.contains("get_connection_status")));
    }

    #[test]
    fn a_grant_without_the_connect_tool_is_sent_to_the_app() {
        let url = connections_page("https://app.dravr.ai/");
        let response =
            external_no_provider_response(&offers(), &ConnectRoute::App(Some(url.clone())));
        let result = response.result.unwrap();
        assert_eq!(url, "https://app.dravr.ai/#settings/connections");
        assert_eq!(result["connect_url"], url.as_str());
        let message = result["message"].as_str().unwrap();
        assert!(message.contains(&url), "{message}");
        assert!(!message.contains("connect_provider"), "{message}");
    }
}
