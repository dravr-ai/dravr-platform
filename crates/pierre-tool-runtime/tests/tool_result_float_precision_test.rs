// ABOUTME: Pins that typed tool results carry each float at its own precision, f32 and f64 alike
// ABOUTME: to_value widened f32 to f64, so a reply read 12.800000190734863 where the data said 12.8
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every typed tool result goes through `ok_typed`, and a `format=toon` one
//! through `apply_format` as well. Both converted the payload with
//! `serde_json::to_value`, which widens an `f32` to `f64`: 12.8 has no exact
//! `f32`, so a weather temperature reached the athlete as 12.800000190734863.
//! The two cases here are the two halves of the fix — `f32` written at its own
//! precision, and `f64` surviving the text round trip bit for bit, which only
//! holds with `serde_json`'s `float_roundtrip`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_formatters::OutputFormat;
use pierre_tool_runtime::conversions::{apply_format, ok_typed};
use serde::Serialize;

#[derive(Serialize)]
struct Sample {
    temperature_celsius: f32,
    humidity_percentage: Option<f32>,
    wind_speed_kmh: Option<f32>,
    distance_km: f64,
}

fn sample() -> Sample {
    Sample {
        temperature_celsius: 12.8,
        humidity_percentage: Some(61.3),
        wind_speed_kmh: None,
        distance_km: 0.1,
    }
}

#[test]
fn an_f32_reaches_the_reply_at_its_own_precision() {
    let result = ok_typed("test_tool", sample()).expect("serializes");
    let text = result.content.to_string();
    // Key order depends on whether serde_json's preserve_order is unified into
    // the build, which this is not about; each value is checked on its own.
    for fragment in [
        r#""temperature_celsius":12.8"#,
        r#""humidity_percentage":61.3"#,
        r#""wind_speed_kmh":null"#,
        r#""distance_km":0.1"#,
    ] {
        assert!(text.contains(fragment), "expected {fragment} in {text}");
    }
    assert!(!text.contains("12.80000"), "an f32 was widened: {text}");
}

#[test]
fn the_toon_path_does_not_widen_either() {
    let formatted = apply_format(sample(), OutputFormat::Toon);
    let text = serde_json::to_string(&formatted).expect("serializes");
    assert!(text.contains("12.8"), "{text}");
    assert!(
        !text.contains("12.80000"),
        "TOON rendered a widened f32: {text}"
    );
}

#[test]
fn every_f64_survives_the_text_round_trip_bit_for_bit() {
    // A deterministic spread of bit patterns across the whole finite range.
    // serde_json's default parser is best-effort and misses some of these by
    // one ULP; `float_roundtrip` makes it exact. NaN and infinities are not
    // JSON numbers and are skipped.
    #[derive(Serialize)]
    struct One {
        value: f64,
    }
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut checked = 0_u32;
    for _ in 0..50_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let value = f64::from_bits(state);
        if !value.is_finite() {
            continue;
        }
        let result = ok_typed("test_tool", One { value }).expect("serializes");
        let back = result.content["value"].as_f64().expect("a number");
        assert_eq!(
            back.to_bits(),
            value.to_bits(),
            "{value:e} came back as {back:e}"
        );
        checked += 1;
    }
    assert!(
        checked > 45_000,
        "the spread must actually exercise the parser, checked {checked}"
    );
}
