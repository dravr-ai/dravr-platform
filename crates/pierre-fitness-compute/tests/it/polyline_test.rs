// ABOUTME: Unit tests for the Google encoded-polyline codec — the reference string, round trips and every refusal
// ABOUTME: A malformed, truncated or off-globe polyline decodes to None, never to the prefix that parsed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_fitness_compute::{decode_polyline, encode_polyline};

/// The Google reference: (38.5,-120.2), (40.7,-120.95), (43.252,-126.453).
const REFERENCE_POLYLINE: &str = "_p~iF~ps|U_ulLnnqC_mqNvxq`@";

const REFERENCE_POINTS: [(f64, f64); 3] = [(38.5, -120.2), (40.7, -120.95), (43.252, -126.453)];

#[test]
fn the_reference_string_decodes_to_the_reference_points() {
    let points = decode_polyline(REFERENCE_POLYLINE).unwrap();
    assert_eq!(points, REFERENCE_POINTS.to_vec());
}

#[test]
fn the_reference_points_encode_to_the_reference_string() {
    assert_eq!(encode_polyline(&REFERENCE_POINTS), REFERENCE_POLYLINE);
}

#[test]
fn a_decoded_route_re_encodes_to_the_same_string() {
    // A Montréal loop at precision 5, negative longitude deltas and all.
    let loop_points: Vec<(f64, f64)> = (0..40)
        .map(|i| {
            let angle = f64::from(i) * 0.157;
            (
                0.01f64.mul_add(angle.sin(), 45.5259),
                0.015f64.mul_add(angle.cos(), -73.5697),
            )
        })
        .collect();
    let encoded = encode_polyline(&loop_points);
    let decoded = decode_polyline(&encoded).unwrap();
    assert_eq!(decoded.len(), loop_points.len());
    for (decoded, original) in decoded.iter().zip(&loop_points) {
        assert!((decoded.0 - original.0).abs() <= 0.000_005);
        assert!((decoded.1 - original.1).abs() <= 0.000_005);
    }
    assert_eq!(encode_polyline(&decoded), encoded);
}

#[test]
fn an_empty_string_is_an_empty_route() {
    assert_eq!(decode_polyline("").unwrap(), Vec::<(f64, f64)>::new());
    assert_eq!(encode_polyline(&[]), "");
}

#[test]
fn a_string_cut_off_mid_value_is_refused() {
    // Dropping the last character leaves the final longitude's continuation
    // chunk with nothing after it.
    let truncated = &REFERENCE_POLYLINE[..REFERENCE_POLYLINE.len() - 1];
    assert!(decode_polyline(truncated).is_none());
}

#[test]
fn a_latitude_without_its_longitude_is_refused() {
    // "_p~iF" is exactly one complete value: 38.5 with no longitude after it.
    assert!(decode_polyline("_p~iF").is_none());
}

#[test]
fn a_character_outside_the_alphabet_is_refused() {
    assert!(decode_polyline("_p~iF ~ps|U").is_none());
    assert!(decode_polyline("_p~iF~ps|U\u{7f}").is_none());
    assert!(decode_polyline("é").is_none());
}

#[test]
fn a_point_off_the_globe_is_refused() {
    let off_globe = encode_polyline(&[(91.0, 10.0)]);
    assert!(decode_polyline(&off_globe).is_none());
    let past_the_antimeridian = encode_polyline(&[(10.0, 181.0)]);
    assert!(decode_polyline(&past_the_antimeridian).is_none());
}

#[test]
fn a_value_longer_than_any_coordinate_is_refused() {
    // Eight continuation chunks then a terminator: far past the six a
    // precision-5 delta can need.
    assert!(decode_polyline("~~~~~~~~?").is_none());
}
