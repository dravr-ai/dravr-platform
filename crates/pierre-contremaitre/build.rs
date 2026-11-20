// ABOUTME: Build script that brotli-compresses the five locale catalogues into OUT_DIR
// ABOUTME: The messaging-strings registry embeds the compressed copies and inflates them at startup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Compresses `packages/i18n/src/locales/<locale>/translation.json` for the
//! registry in `src/messaging_strings.rs`.
//!
//! The five catalogues are ~1.5 MB of pretty-printed JSON, and embedding them
//! as text put all of it in the shipped binary. Compressed at quality 9 they
//! take ~370 KB; the registry inflates them once, when it is seeded
//! (carnet#1645). The decoder is the brotli one the server already links for
//! response compression.

use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::PathBuf;

/// The catalogue's locales, in the order the registry seeds them. The
/// registry's `CATALOGUE` names the same five files.
const LOCALES: [&str; 5] = ["fr", "en", "es", "de", "pt"];

/// Brotli quality: 9 is within 8% of the maximum (11) on these files and
/// compresses them in a fraction of the time, which every rebuild of this
/// crate pays.
const QUALITY: u32 = 9;

/// Brotli window, log2 of its size in bytes (22 = 4 MiB, the format's
/// default), larger than any one catalogue.
const WINDOW_LOG2: u32 = 22;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let locales_dir = manifest_dir.join("../../packages/i18n/src/locales");
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);

    for locale in LOCALES {
        let source = locales_dir.join(locale).join("translation.json");
        println!("cargo:rerun-if-changed={}", source.display());
        let json = fs::read(&source)
            .map_err(|e| format!("cannot read catalogue {}: {e}", source.display()))?;
        let mut encoder = brotli::CompressorWriter::new(Vec::new(), 4096, QUALITY, WINDOW_LOG2);
        encoder.write_all(&json)?;
        encoder.flush()?;
        fs::write(
            out_dir.join(format!("{locale}.translation.json.br")),
            encoder.into_inner(),
        )?;
    }
    Ok(())
}
