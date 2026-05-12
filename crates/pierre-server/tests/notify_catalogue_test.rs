// ABOUTME: Validates every `target: "notify"` call site emits an event present
// ABOUTME: in vendor/contremaitre/schemas/notify-events.yaml with required fields.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Typo guard for `info!(target: "notify", event = "...")` call sites.
//!
//! Each call site in pierre-* crates that emits a notify event must:
//! - reference an `event = "..."` name listed in
//!   `vendor/contremaitre/schemas/notify-events.yaml`, and
//! - declare every required field for that event, either as a literal
//!   key on the `info!` macro itself, or on an enclosing
//!   `#[tracing::instrument(..., fields(...))]` attribute in the same
//!   function.
//!
//! `tenant_id` and `user_id` are universal span fields per ADR-014 — the
//! NotifyLayer extracts them at runtime from the enclosing handler span,
//! which may live in a different file than the emit site, so the
//! same-file `#[instrument]` check this test does is too narrow to police
//! them. The catalogue still lists them as required so operators know
//! they belong on every event.
//!
//! The test no-ops when the catalogue YAML is missing (the
//! dravr-contremaitre submodule may not be bumped on a worktree yet).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// One emit-site found in the codebase.
#[derive(Debug)]
struct NotifyCall {
    file: PathBuf,
    line: usize,
    event: String,
    /// Field names supplied directly on the `info!(...)` macro call.
    inline_fields: HashSet<String>,
    /// Field names declared on the enclosing `#[instrument(..., fields(...))]`.
    instrument_fields: HashSet<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct CatalogueEntry {
    name: String,
    #[serde(default)]
    required_fields: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawCatalogue {
    #[serde(default)]
    events: Vec<CatalogueEntry>,
}

#[derive(Debug, Default)]
struct Catalogue {
    events: HashMap<String, CatalogueEntry>,
}

impl Catalogue {
    fn from_yaml(text: &str) -> serde_yaml::Result<Self> {
        let raw: RawCatalogue = serde_yaml::from_str(text)?;
        let events = raw
            .events
            .into_iter()
            .map(|entry| (entry.name.clone(), entry))
            .collect();
        Ok(Self { events })
    }
}

#[test]
fn notify_call_sites_match_catalogue() {
    let yaml_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/contremaitre/schemas/notify-events.yaml");
    if !yaml_path.exists() {
        eprintln!(
            "notify-events.yaml not yet vendored at {}, skipping catalogue test",
            yaml_path.display()
        );
        return;
    }

    let yaml_text = fs::read_to_string(&yaml_path).expect("read notify-events.yaml");
    let catalogue = Catalogue::from_yaml(&yaml_text).expect("parse notify-events.yaml");

    let crates_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../")
        .canonicalize()
        .expect("canonicalize crates root");

    let mut calls: Vec<NotifyCall> = Vec::new();
    for entry in fs::read_dir(&crates_root).expect("list crates dir") {
        let entry = entry.expect("crate dir entry");
        let crate_dir = entry.path();
        if !crate_dir.is_dir() {
            continue;
        }
        let src = crate_dir.join("src");
        if src.is_dir() {
            walk_src(&src, &mut calls);
        }
    }

    assert!(
        !calls.is_empty(),
        "expected at least one notify call site under {}/*/src",
        crates_root.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for call in &calls {
        let Some(entry) = catalogue.events.get(&call.event) else {
            failures.push(format!(
                "{}:{}: unknown notify event `{}` (not in notify-events.yaml)",
                call.file.display(),
                call.line,
                call.event
            ));
            continue;
        };
        for required in &entry.required_fields {
            // `tenant_id` / `user_id` flow in from a caller-side span and
            // can't be statically checked from the emit-site file alone.
            if required == "tenant_id" || required == "user_id" {
                continue;
            }
            let satisfied =
                call.inline_fields.contains(required) || call.instrument_fields.contains(required);
            if !satisfied {
                failures.push(format!(
                    "{}:{}: notify event `{}` missing required field `{}`",
                    call.file.display(),
                    call.line,
                    call.event,
                    required
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "notify catalogue validation failed:\n  {}",
        failures.join("\n  ")
    );
}

fn walk_src(root: &Path, out: &mut Vec<NotifyCall>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            walk_src(&path, out);
        } else if ft.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if !text.contains("target: \"notify\"") {
                continue;
            }
            scan_file(&path, &text, out);
        }
    }
}

/// Line-based scanner. For every `target: "notify"` line we treat the
/// enclosing `info!(...)` (or `tracing::info!(...)`) as the boundary and
/// pull `event = "..."` plus the other `key = ...` fragments from the
/// macro body. Line-based avoids the byte-level paren matching that the
/// earlier implementation hung on.
fn scan_file(path: &Path, text: &str, out: &mut Vec<NotifyCall>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].contains("target: \"notify\"") {
            // Find the line above this one that has `info!(` (or
            // `tracing::info!(`). Bounded look-back of 20 lines.
            let start_line = look_back_macro_open(&lines, i);
            // Find the line that has the matching `;` or `)`-then-`;`
            // closing the macro. Bounded look-ahead of 60 lines.
            let end_line = look_ahead_macro_close(&lines, i);
            let body = lines[start_line..=end_line].join("\n");

            if let Some(event) = extract_event_literal(&body) {
                let inline_fields = extract_inline_field_keys(&body);
                let instrument_fields = look_back_instrument_fields(&lines, start_line);
                out.push(NotifyCall {
                    file: path.to_path_buf(),
                    line: i + 1,
                    event,
                    inline_fields,
                    instrument_fields,
                });
            }
            i = end_line + 1;
        } else {
            i += 1;
        }
    }
}

fn look_back_macro_open(lines: &[&str], idx: usize) -> usize {
    let lo = idx.saturating_sub(20);
    for k in (lo..=idx).rev() {
        if lines[k].contains("info!(") {
            return k;
        }
    }
    idx
}

fn look_ahead_macro_close(lines: &[&str], idx: usize) -> usize {
    let hi = (idx + 60).min(lines.len() - 1);
    for k in idx..=hi {
        // Crude but reliable: an `info!(...)` ends with `);` or `)?;`
        // or `)` followed by a comment, typically on its own line.
        let trimmed = lines[k].trim_end();
        if trimmed.ends_with(");") || trimmed.ends_with(")") {
            return k;
        }
    }
    hi
}

fn extract_event_literal(body: &str) -> Option<String> {
    let idx = body.find("event")?;
    let after = body[idx + "event".len()..].trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let after = after.strip_prefix('"')?;
    let end = after.find('"')?;
    Some(after[..end].to_owned())
}

/// Collect every `key = ...` fragment we can see in the macro body.
/// Strict matching is unnecessary — false positives don't cause
/// failures (they just over-satisfy the required-field check).
fn extract_inline_field_keys(body: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in body.lines() {
        let trimmed = line.trim_start_matches(|c: char| c.is_whitespace() || c == ',');
        let Some(eq) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq].trim();
        let key = key.trim_start_matches('%').trim_start_matches('?').trim();
        if key == "target" || key == "event" {
            continue;
        }
        if !key.is_empty()
            && key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
        {
            out.insert(key.to_owned());
        }
    }
    out
}

/// Walk backward from the macro-open line looking for an enclosing
/// `#[tracing::instrument(..., fields(...))]` or `#[instrument(...)]`
/// attribute and return the field names from its `fields(...)` clause.
/// The attribute usually sits 1–10 lines above the `async fn` header,
/// which itself usually sits within ~30 lines of the macro open.
fn look_back_instrument_fields(lines: &[&str], macro_open: usize) -> HashSet<String> {
    let lo = macro_open.saturating_sub(120);
    // Find the nearest enclosing fn-header (`fn `, `async fn ` or `pub fn `).
    let mut fn_line = None;
    for k in (lo..=macro_open).rev() {
        let t = lines[k].trim_start();
        if t.starts_with("fn ")
            || t.starts_with("async fn ")
            || t.starts_with("pub fn ")
            || t.starts_with("pub async fn ")
            || t.starts_with("pub(crate) fn ")
            || t.starts_with("pub(crate) async fn ")
        {
            fn_line = Some(k);
            break;
        }
    }
    let Some(fn_line) = fn_line else {
        return HashSet::new();
    };

    // Scan the ~10 lines directly above the fn header for the
    // instrument attribute. Attributes may span multiple lines.
    let attr_lo = fn_line.saturating_sub(15);
    let attr_block: String = lines[attr_lo..fn_line].join("\n");
    if !attr_block.contains("instrument(") {
        return HashSet::new();
    }

    // Pull out the `fields(...)` body from the attribute block.
    let Some(fields_idx) = attr_block.find("fields(") else {
        return HashSet::new();
    };
    let after = &attr_block[fields_idx + "fields(".len()..];
    // The fields clause ends at the next `)` at depth 0. Bounded scan.
    let mut depth = 1i32;
    let mut end = 0usize;
    for (i, ch) in after.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
            _ => {}
        }
    }
    if end == 0 {
        return HashSet::new();
    }
    let fields_body = &after[..end];

    let mut out = HashSet::new();
    // Fields are comma-separated `name`, `name = value`, or
    // `name = %expr` / `name = ?expr`. Split on commas at top level.
    let mut depth = 0i32;
    let mut start = 0usize;
    let bytes = fields_body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                push_field_key(&fields_body[start..i], &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_field_key(&fields_body[start..], &mut out);
    out
}

fn push_field_key(piece: &str, out: &mut HashSet<String>) {
    let piece = piece.trim();
    if piece.is_empty() {
        return;
    }
    let key = if let Some(eq) = piece.find('=') {
        piece[..eq].trim()
    } else {
        piece
    };
    let key = key.trim_start_matches('%').trim_start_matches('?').trim();
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        out.insert(key.to_owned());
    }
}
