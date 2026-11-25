// ABOUTME: def_sig: Dravr's Ed25519 signature over each served tool definition's def_hash, signed once per hash
// ABOUTME: The record_log checkpoint key signs it, domain-separated from checkpoint notes; no signer, no def_sig
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `def_sig` on every published tool definition (carnet#1543, design row 38)
//!
//! Every definition `tools/list` and `GET /mcp/tools` serve carries its
//! `def_hash` (dravr-tronc, `Tool::def_hash`). This module adds Dravr's
//! signature over that hash beside it, at
//! `_meta["ai.dravr.vault/tool"].def_sig`, so a client that pins a definition
//! can also check that Dravr served it.
//!
//! ## What is signed
//!
//! The UTF-8 bytes of `TOOL_DEF_SIG_CONTEXT` (`"dravr.ai/tool-def\n"`)
//! followed by the served `def_hash` string exactly as published
//! (`sha256:` + 64 lowercase hex digits), nothing after it. The value is
//! `"ed25519:"` + the standard (padded) base64 of the 64-byte Ed25519
//! signature, verifiable under the checkpoint key Dravr publishes at
//! `/.well-known/dravr-checkpoint` and in `/.well-known/did.json`.
//!
//! ## Why the key can sign both
//!
//! The key is the `record_log` checkpoint signer
//! ([`pierre_services::barrage_keys::checkpoint_signer`]). It signs two
//! other message families:
//!
//! - a checkpoint note body, which barrage's `cut_checkpoint` writes as
//!   `"<origin>\n<size>\n<root>\n"` with the origin
//!   [`pierre_services::barrage_keys::CHECKPOINT_ORIGIN`]
//!   (`dravr.ai/barrage-log`): its first line is never `dravr.ai/tool-def`;
//! - a consent grant receipt's compact-JWS signing input
//!   ([`pierre_services::grant_receipt_jws`], carnet#1544),
//!   `BASE64URL(header) '.' BASE64URL(payload)`: it starts with `eyJ` (the
//!   base64url of `{"`) and holds no newline, so it has no first line of
//!   either form.
//!
//! So no tool-definition message is a message of another family, and no
//! message of another family is a tool-definition message
//! (`grant_receipt_jws`'s domain-separation test pins all three apart).
//!
//! ## When it is signed
//!
//! A caller offered fewer arguments (a coach tool listed without `athlete`)
//! is served another definition with its own `def_hash`, so the signature is
//! taken over the hash actually served, at the end of the dispatcher's
//! `list_tools`, the one place both publishers read. Signing is a Cloud KMS
//! call there, so each distinct hash is signed once per process and kept
//! ([`ToolDefSignatures`]); the first listing after a boot or a description
//! change signs its misses concurrently, within a short budget
//! (`SIGNING_BUDGET`). A signature that fails or overruns is left off
//! that definition for that listing (and retried on the next), never replaced
//! by a stand-in: a deployment with no signer publishes no `def_sig` at all.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use dravr_barrage::keyed::Signer;
use dravr_tronc::mcp::schema::{Tool, TOOL_META_KEY};
use futures_util::stream::{self, StreamExt};
use pierre_core::errors::{AppError, AppResult};
use pierre_services::barrage_keys::TOOL_DEF_SIG_CONTEXT;
use serde_json::{Map, Value};
use tokio::time::{self, Instant};
use tracing::warn;

/// The field of the `_meta["ai.dravr.vault/tool"]` object that carries the signature.
///
/// Its value is `"ed25519:"` + standard base64 of the 64-byte Ed25519
/// signature over `TOOL_DEF_SIG_CONTEXT` followed by the `def_hash` served
/// beside it.
const DEF_SIG_FIELD: &str = "def_sig";

/// The algorithm prefix of a `def_sig` value.
const DEF_SIG_PREFIX: &str = "ed25519:";

/// How long a listing waits for the signatures it misses. A Cloud KMS
/// signature takes tens of milliseconds; the client's own timeout is 15 s,
/// which a listing must not inherit while KMS is down, since a failure is not
/// kept and every listing would wait it out. Past the budget the listing goes
/// out without those `def_sig`s and the next one signs them.
const SIGNING_BUDGET: Duration = Duration::from_secs(2);

/// How many signatures one listing asks for at once: a cold listing misses
/// every definition, and the KMS client is shared with the checkpoint cut.
const MAX_CONCURRENT_SIGNATURES: usize = 16;

/// How many signatures a process keeps before it starts over. Each tool
/// variant has one hash until its description changes, so a process holds a
/// few hundred; the bound only stops a long run of description reloads
/// growing the map without end.
const MAX_KEPT_SIGNATURES: usize = 4096;

/// The signer of this process's tool definitions and the signatures it has
/// made, keyed by the `def_hash` they sign.
///
/// Empty until the composition root installs the checkpoint signer
/// ([`Self::install`]), before the server answers its first request; a
/// deployment that runs none (a KMS deployment naming no signing key, an
/// in-memory database) never installs one and publishes no `def_sig`.
#[derive(Default)]
pub struct ToolDefSignatures {
    signer: OnceLock<Arc<dyn Signer>>,
    signed: RwLock<HashMap<String, String>>,
}

impl fmt::Debug for ToolDefSignatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolDefSignatures")
            .field("signing", &self.signer.get().is_some())
            .finish_non_exhaustive()
    }
}

impl ToolDefSignatures {
    /// Sign this process's tool definitions with `signer`, the checkpoint
    /// signer the boot built.
    ///
    /// The same key installed again is a no-op.
    ///
    /// # Errors
    ///
    /// When another key is already installed: a definition signed by one key
    /// and served beside a published key that is another would verify
    /// against nothing.
    pub fn install(&self, signer: Arc<dyn Signer>) -> AppResult<()> {
        let public_key = signer.public_key();
        let installed = self.signer.get_or_init(|| signer);
        if installed.public_key() == public_key {
            return Ok(());
        }
        Err(AppError::internal(
            "a second tool-definition signing key was offered; a process signs its tool \
             definitions with the one checkpoint key it publishes",
        ))
    }

    /// `tools` with each definition's `def_sig` written beside the
    /// `def_hash` tronc stamps on it; `tools` unchanged without a signer.
    pub(crate) async fn stamp(&self, tools: Vec<Tool>) -> Vec<Tool> {
        let Some(signer) = self.signer.get() else {
            return tools;
        };
        let hashes: Vec<String> = tools.iter().map(Tool::def_hash).collect();
        let mut known = self.known(&hashes);
        let missing: Vec<String> = hashes
            .iter()
            .filter(|hash| !known.contains_key(*hash))
            .collect::<HashSet<_>>()
            .into_iter()
            .cloned()
            .collect();
        let fresh = sign_missing(signer, missing).await;
        self.keep(&fresh);
        known.extend(fresh);
        tools
            .into_iter()
            .zip(hashes)
            .map(|(tool, hash)| match known.get(&hash) {
                Some(signature) => with_def_sig(tool, signature.clone()),
                None => tool,
            })
            .collect()
    }

    /// The kept signatures of `hashes`.
    fn known(&self, hashes: &[String]) -> HashMap<String, String> {
        let signed = self.signed.read().unwrap_or_else(PoisonError::into_inner);
        hashes
            .iter()
            .filter_map(|hash| Some((hash.clone(), signed.get(hash)?.clone())))
            .collect()
    }

    /// Keep `fresh`, starting over once the map is full.
    fn keep(&self, fresh: &[(String, String)]) {
        if fresh.is_empty() {
            return;
        }
        let mut signed = self.signed.write().unwrap_or_else(PoisonError::into_inner);
        if signed.len() + fresh.len() > MAX_KEPT_SIGNATURES {
            signed.clear();
        }
        signed.extend(fresh.iter().cloned());
    }
}

/// The message a `def_sig` signs: `TOOL_DEF_SIG_CONTEXT` then `def_hash`.
fn signed_message(def_hash: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(TOOL_DEF_SIG_CONTEXT.len() + def_hash.len());
    message.extend_from_slice(TOOL_DEF_SIG_CONTEXT.as_bytes());
    message.extend_from_slice(def_hash.as_bytes());
    message
}

/// The signatures of `missing` that `signer` makes within
/// [`SIGNING_BUDGET`], at most [`MAX_CONCURRENT_SIGNATURES`] at a time.
///
/// Every signature finished by the deadline is returned, so one stalled
/// call costs only its own definition's `def_sig`; a failure or an overrun
/// is logged and left out, to be signed by a later listing.
async fn sign_missing(signer: &Arc<dyn Signer>, missing: Vec<String>) -> Vec<(String, String)> {
    let deadline = Instant::now() + SIGNING_BUDGET;
    let signer = Arc::clone(signer);
    let mut signing = stream::iter(missing)
        .map(move |hash| {
            let signer = Arc::clone(&signer);
            async move {
                let answer = sign_def_hash(signer.as_ref(), &hash).await;
                (hash, answer)
            }
        })
        .buffer_unordered(MAX_CONCURRENT_SIGNATURES);
    let mut fresh = Vec::new();
    loop {
        match time::timeout_at(deadline, signing.next()).await {
            Ok(Some((hash, Ok(signature)))) => fresh.push((hash, signature)),
            Ok(Some((hash, Err(e)))) => {
                warn!(def_hash = %hash, error = %e, "a tool definition was listed without its def_sig");
            }
            Ok(None) => break,
            Err(_) => {
                warn!(
                    budget_ms = SIGNING_BUDGET.as_millis(),
                    "tool definitions were listed without their def_sig: signing overran its budget"
                );
                break;
            }
        }
    }
    fresh
}

/// The `def_sig` value of `def_hash` under `signer`.
async fn sign_def_hash(signer: &dyn Signer, def_hash: &str) -> AppResult<String> {
    let signature = signer
        .sign(&signed_message(def_hash))
        .await
        .map_err(|e| AppError::internal(format!("tool definition signature: {e}")))?;
    Ok(format!("{DEF_SIG_PREFIX}{}", BASE64.encode(signature)))
}

/// `tool` with `signature` at `_meta["ai.dravr.vault/tool"].def_sig`, every
/// other `_meta` key and claim kept. `_meta` is outside what `def_hash`
/// covers, so the hash tronc stamps afterwards is the one signed here.
fn with_def_sig(mut tool: Tool, signature: String) -> Tool {
    let meta = tool.meta.get_or_insert_with(Map::new);
    let entry = meta
        .entry(TOOL_META_KEY.to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    if let Value::Object(claims) = entry {
        claims.insert(DEF_SIG_FIELD.to_owned(), Value::String(signature));
    }
    tool
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use async_trait::async_trait;
    use dravr_barrage::digest::Sha256;
    use dravr_barrage::record_log::checkpoint::note_body;
    use dravr_barrage::BarrageError;
    use dravr_tronc::mcp::schema::DEF_HASH_FIELD;
    use pierre_services::barrage_keys::{LocalSigner, CHECKPOINT_ORIGIN};
    use ring::signature::{UnparsedPublicKey, ED25519};
    use serde_json::json;
    use tokio::time;

    use super::*;

    fn tool(name: &str) -> Tool {
        Tool::new(name, "Reads a thing.", json!({"type": "object"}))
    }

    fn def_sig_of(tool: &Tool) -> Option<&str> {
        tool.meta
            .as_ref()?
            .get(TOOL_META_KEY)?
            .get(DEF_SIG_FIELD)?
            .as_str()
    }

    /// What a [`Counting`] signer does with each message.
    enum Answer {
        Sign,
        Fail,
        /// Stall on a message holding this text (every message holds "").
        StallOn(String),
    }

    /// A signer counting its signatures, answering each as `answer` says.
    struct Counting {
        inner: LocalSigner,
        calls: AtomicUsize,
        answer: Answer,
    }

    #[async_trait]
    impl Signer for Counting {
        fn key_name(&self) -> &str {
            self.inner.key_name()
        }

        fn public_key(&self) -> [u8; 32] {
            self.inner.public_key()
        }

        async fn sign(&self, message: &[u8]) -> Result<[u8; 64], BarrageError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match &self.answer {
                Answer::Sign => self.inner.sign(message).await,
                Answer::Fail => Err(BarrageError::invalid("signer", "KMS unreachable")),
                Answer::StallOn(text) => {
                    if String::from_utf8_lossy(message).contains(text.as_str()) {
                        time::sleep(Duration::from_secs(60)).await;
                    }
                    self.inner.sign(message).await
                }
            }
        }
    }

    fn counting(answer: Answer) -> Arc<Counting> {
        Arc::new(Counting {
            inner: LocalSigner::random().unwrap(),
            calls: AtomicUsize::new(0),
            answer,
        })
    }

    #[test]
    fn a_tool_definition_message_is_never_a_checkpoint_note() {
        let note = note_body(CHECKPOINT_ORIGIN, 7, &Sha256::of(b"root"));
        assert!(
            !note.as_bytes().starts_with(TOOL_DEF_SIG_CONTEXT.as_bytes()),
            "bug: a checkpoint note begins like a tool-definition message"
        );
        assert!(
            !signed_message("sha256:00").starts_with(format!("{CHECKPOINT_ORIGIN}\n").as_bytes()),
            "bug: a tool-definition message begins like a checkpoint note"
        );
    }

    #[tokio::test]
    async fn a_def_sig_verifies_over_the_context_and_the_served_hash() {
        let signatures = ToolDefSignatures::default();
        let signer = counting(Answer::Sign);
        signatures.install(signer.clone()).unwrap();
        let listed = tool("get_activities");
        let hash = listed.def_hash();
        let stamped = signatures
            .stamp(vec![listed])
            .await
            .remove(0)
            .with_def_hash();
        let value = def_sig_of(&stamped).expect("bug: a signed listing carries no def_sig");
        let signature = BASE64
            .decode(
                value
                    .strip_prefix("ed25519:")
                    .expect("bug: def_sig is not ed25519:"),
            )
            .unwrap();
        let mut message = b"dravr.ai/tool-def\n".to_vec();
        message.extend_from_slice(hash.as_bytes());
        assert!(
            UnparsedPublicKey::new(&ED25519, signer.public_key())
                .verify(&message, &signature)
                .is_ok(),
            "bug: def_sig does not verify over the context and the served hash"
        );
        assert_eq!(
            stamped.meta.as_ref().unwrap()[TOOL_META_KEY][DEF_HASH_FIELD].as_str(),
            Some(hash.as_str()),
            "bug: adding def_sig changed the hash tronc stamps"
        );
    }

    #[tokio::test]
    async fn each_distinct_hash_is_signed_once() {
        let signatures = ToolDefSignatures::default();
        let signer = counting(Answer::Sign);
        signatures.install(signer.clone()).unwrap();
        signatures
            .stamp(vec![tool("a"), tool("b"), tool("a")])
            .await;
        signatures.stamp(vec![tool("a"), tool("b")]).await;
        assert_eq!(
            signer.calls.load(Ordering::SeqCst),
            2,
            "bug: a hash already signed was signed again"
        );
    }

    #[tokio::test]
    async fn a_failed_signature_publishes_no_def_sig_and_is_retried() {
        let signatures = ToolDefSignatures::default();
        let signer = counting(Answer::Fail);
        signatures.install(signer.clone()).unwrap();
        let first = signatures.stamp(vec![tool("a")]).await;
        assert_eq!(
            def_sig_of(&first[0]),
            None,
            "bug: a failed signature published a def_sig"
        );
        signatures.stamp(vec![tool("a")]).await;
        assert_eq!(
            signer.calls.load(Ordering::SeqCst),
            2,
            "bug: a failed signature was kept instead of retried"
        );
    }

    #[tokio::test]
    async fn a_stalled_signature_costs_only_its_own_def_sig() {
        let stalled = tool("stalled");
        let signer = counting(Answer::StallOn(stalled.def_hash()));
        let signatures = ToolDefSignatures::default();
        signatures.install(signer.clone()).unwrap();
        let started = Instant::now();
        let listed = signatures.stamp(vec![stalled, tool("signed")]).await;
        assert!(
            started.elapsed() < SIGNING_BUDGET + Duration::from_secs(5),
            "bug: a stalled signer held the listing past its budget"
        );
        assert_eq!(
            def_sig_of(&listed[0]),
            None,
            "bug: an unsigned definition carries a def_sig"
        );
        assert!(
            def_sig_of(&listed[1]).is_some(),
            "bug: one stalled signature dropped another that finished"
        );
        let calls = signer.calls.load(Ordering::SeqCst);
        signatures.stamp(vec![tool("signed")]).await;
        assert_eq!(
            signer.calls.load(Ordering::SeqCst),
            calls,
            "bug: a signature finished beside a stalled one was not kept"
        );
    }

    #[tokio::test]
    async fn no_signer_publishes_no_def_sig() {
        let stamped = ToolDefSignatures::default().stamp(vec![tool("a")]).await;
        assert!(
            stamped[0].meta.is_none(),
            "bug: an unsigned listing carries _meta"
        );
    }

    #[test]
    fn a_second_signing_key_is_refused() {
        let signatures = ToolDefSignatures::default();
        let first = counting(Answer::Sign);
        signatures.install(first.clone()).unwrap();
        assert!(
            signatures.install(first).is_ok(),
            "bug: the same key offered again is refused"
        );
        assert!(
            signatures.install(counting(Answer::Sign)).is_err(),
            "bug: a second key was accepted silently"
        );
    }
}
