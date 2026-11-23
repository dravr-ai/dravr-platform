// ABOUTME: Every served tool definition's def_sig verifies under the deployment's checkpoint key over the documented bytes
// ABOUTME: tools/list and GET /mcp/tools agree, a narrowed definition carries its own signature, no signer publishes none

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! # `def_sig` on the served definitions (carnet#1543, design row 38)
//!
//! A definition is signed by the `record_log` checkpoint key over the UTF-8
//! text `dravr.ai/tool-def\n` followed by the `def_hash` served beside it,
//! and published as `"ed25519:" + base64` at
//! `_meta["ai.dravr.vault/tool"].def_sig`. The bytes are spelled out here
//! rather than taken from the server, so a change to them fails this suite.

#[cfg(all(feature = "tools-groups", feature = "tools-memory"))]
mod tool_def_sig_tests {
    use crate::common::{self, create_test_server_resources};
    use crate::helpers::coach_roster_fixtures::{
        coach, coached_group, group_agent, person, Person,
    };

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use dravr_barrage::keyed::Signer as _;
    use dravr_tronc::mcp::schema::{Tool, DEF_HASH_FIELD, TOOL_META_KEY};
    use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_services::barrage_keys::{LocalSigner, PlatformKeyed};
    use ring::signature::{UnparsedPublicKey, ED25519};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    /// A read tool a coach is listed with `athlete` and anyone else without.
    const SCOPED_READ: &str = "get_activities";

    fn grant(res: &Arc<ServerContext>, user: &Person) -> String {
        format!(
            "Bearer {}",
            common::delegated_test_token_for_tenant(res, user.id, Some(user.tenant.to_string()))
        )
    }

    async fn send(app: &Router, request: Request<Body>) -> Value {
        let response = app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        serde_json::from_slice(&bytes).unwrap()
    }

    /// The `tools/list` answer over JSON-RPC.
    async fn tools_list(app: &Router, authorization: &str) -> Vec<Value> {
        let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
        let request = Request::post("/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .header("authorization", authorization)
            .body(Body::from(body.to_string()))
            .unwrap();
        let listed = send(app, request).await;
        listed["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list answered: {listed}"))
            .clone()
    }

    /// The `GET /mcp/tools` discovery answer.
    async fn discovery(app: &Router, authorization: &str) -> Vec<Value> {
        let request = Request::get("/mcp/tools")
            .header("authorization", authorization)
            .body(Body::empty())
            .unwrap();
        send(app, request).await["tools"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// Install the signer a deployment without KMS boots with: the local key
    /// derived from this database (`checkpoint_signer`'s local branch), whose
    /// public key is the one it publishes. Built directly rather than through
    /// `checkpoint_signer`, which publishes into a process-wide slot that a
    /// second test database, holding another key, would be refused.
    fn sign_with_the_checkpoint_key(res: &Arc<ServerContext>) -> [u8; 32] {
        let signer = LocalSigner::derived(&PlatformKeyed::of(&res.agent.database)).unwrap();
        let public_key = signer.public_key();
        res.mcp.tool_def_sigs.install(Arc::new(signer)).unwrap();
        public_key
    }

    fn claims(served: &Value) -> &serde_json::Map<String, Value> {
        served["_meta"][TOOL_META_KEY]
            .as_object()
            .unwrap_or_else(|| panic!("{} carries no {TOOL_META_KEY}", served["name"]))
    }

    /// The `def_hash` of `served`, after checking it is its definition's.
    fn def_hash(served: &Value) -> String {
        let hash = claims(served)[DEF_HASH_FIELD].as_str().unwrap().to_owned();
        let definition: Tool = serde_json::from_value(served.clone()).unwrap();
        assert_eq!(
            hash,
            definition.def_hash(),
            "{} is served with another definition's hash",
            served["name"]
        );
        hash
    }

    /// Check `served`'s `def_sig` under `public_key` over exactly
    /// `dravr.ai/tool-def\n` + its served `def_hash`; returns the pair.
    fn verified(served: &Value, public_key: &[u8; 32]) -> (String, String) {
        let name = served["name"].as_str().unwrap();
        let hash = def_hash(served);
        let sig = claims(served)
            .get("def_sig")
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name} is served without a def_sig"))
            .to_owned();
        let signature = BASE64
            .decode(
                sig.strip_prefix("ed25519:")
                    .unwrap_or_else(|| panic!("{name}'s def_sig is not ed25519: {sig}")),
            )
            .unwrap();
        assert_eq!(signature.len(), 64, "{name}'s def_sig is not 64 bytes");
        let mut message = b"dravr.ai/tool-def\n".to_vec();
        message.extend_from_slice(hash.as_bytes());
        assert!(
            UnparsedPublicKey::new(&ED25519, public_key)
                .verify(&message, &signature)
                .is_ok(),
            "{name}'s def_sig does not verify under the checkpoint key"
        );
        (hash, sig)
    }

    fn by_name(tools: &[Value], public_key: &[u8; 32]) -> BTreeMap<String, (String, String)> {
        tools
            .iter()
            .map(|tool| {
                (
                    tool["name"].as_str().unwrap().to_owned(),
                    verified(tool, public_key),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn both_listings_serve_one_def_sig_that_verifies_under_the_checkpoint_key() {
        let res = create_test_server_resources().await.unwrap();
        let public_key = sign_with_the_checkpoint_key(&res);
        let sam = person(&res, "sam@def-sig.test", "Sam Athlete").await;
        let app = ProviderToolRouter::build_http_app(&res);
        let authorization = grant(&res, &sam);

        let listed = by_name(&tools_list(&app, &authorization).await, &public_key);
        assert!(
            listed.contains_key(SCOPED_READ) && listed.len() > 50,
            "premise: the catalogue is listed, or this proves nothing: {:?}",
            listed.keys()
        );
        let discovered = by_name(&discovery(&app, &authorization).await, &public_key);
        assert_eq!(
            listed, discovered,
            "tools/list and GET /mcp/tools serve another hash or signature for one definition"
        );
    }

    #[tokio::test]
    async fn a_narrowed_definition_carries_its_own_valid_signature() {
        let res = create_test_server_resources().await.unwrap();
        let public_key = sign_with_the_checkpoint_key(&res);
        let casey = coach(&res, "casey@def-sig.test", "Casey Coach").await;
        let agent = group_agent(&res, &casey).await;
        coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        let sam = person(&res, "sam2@def-sig.test", "Sam Athlete").await;
        let app = ProviderToolRouter::build_http_app(&res);

        let scoped = |tools: Vec<Value>| {
            tools
                .into_iter()
                .find(|tool| tool["name"] == SCOPED_READ)
                .unwrap_or_else(|| panic!("{SCOPED_READ} is not listed"))
        };
        let coach_view = scoped(tools_list(&app, &grant(&res, &casey)).await);
        let athlete_view = scoped(tools_list(&app, &grant(&res, &sam)).await);
        assert!(
            coach_view["inputSchema"]["properties"]
                .get("athlete")
                .is_some()
                && athlete_view["inputSchema"]["properties"]
                    .get("athlete")
                    .is_none(),
            "premise: the coach is listed athlete= and the athlete is not"
        );
        let (coach_hash, coach_sig) = verified(&coach_view, &public_key);
        let (athlete_hash, athlete_sig) = verified(&athlete_view, &public_key);
        assert_ne!(
            coach_hash, athlete_hash,
            "a narrowed definition is served the hash of the full one"
        );
        assert_ne!(
            coach_sig, athlete_sig,
            "a narrowed definition is served the signature of the full one"
        );
    }

    #[tokio::test]
    async fn without_a_signer_no_definition_carries_a_def_sig() {
        let res = create_test_server_resources().await.unwrap();
        let sam = person(&res, "sam3@def-sig.test", "Sam Athlete").await;
        let app = ProviderToolRouter::build_http_app(&res);
        let authorization = grant(&res, &sam);

        let listed = tools_list(&app, &authorization).await;
        let discovered = discovery(&app, &authorization).await;
        assert!(listed.len() > 50, "premise: the catalogue is listed");
        let served_by_both = listed.iter().chain(&discovered);
        for served in served_by_both {
            def_hash(served);
            assert!(
                !claims(served).contains_key("def_sig"),
                "{} carries a def_sig with no signer installed",
                served["name"]
            );
        }
    }
}
