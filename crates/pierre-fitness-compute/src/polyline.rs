// ABOUTME: Google encoded-polyline codec at precision 5 — the route overview format Strava sends as summary_polyline
// ABOUTME: Decoding refuses a malformed, truncated or off-globe string rather than returning part of a route

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Encoded polylines
//!
//! A provider's activity list can carry the route as a Google encoded
//! polyline: each coordinate is the delta from the previous one, scaled to
//! five decimal places, zig-zag encoded and written as 5-bit chunks offset
//! into printable ASCII (`?` through `~`). Strava's `map.summary_polyline` is
//! exactly this.
//!
//! [`decode_polyline`] is strict: a string that ends mid-value, carries a
//! latitude without its longitude, holds a character outside the alphabet or
//! walks off the globe decodes to `None`, never to the prefix that did parse.
//! Half a route drawn as if it were the whole one is worse than no route.

/// Five decimal places: the precision Strava and the Google reference encode at.
const PRECISION_FACTOR: f64 = 100_000.0;

/// The offset every encoded chunk carries, putting it in printable ASCII.
const CHUNK_OFFSET: u8 = 63;

/// The bit that marks a chunk as followed by another one of the same value.
const CONTINUATION_BIT: i64 = 0x20;

/// The five payload bits of a chunk.
const CHUNK_MASK: i64 = 0x1f;

/// Bits in one chunk's payload.
const CHUNK_BITS: u32 = 5;

/// The widest shift a value's chunks may reach. A coordinate delta at
/// precision 5 is at most 36 000 000, which zig-zags into 27 bits — six
/// chunks — so a seventh continuation is a malformed string rather than a
/// value to accumulate.
const MAX_SHIFT: u32 = 30;

/// Decode an encoded polyline into `(latitude, longitude)` pairs in degrees.
///
/// An empty string is an empty route. Returns `None` when the string is
/// malformed: a character outside `?`..=`~`, a value cut off before its final
/// chunk, an odd number of values (a latitude with no longitude), a value too
/// long for any coordinate, or a point off the globe.
#[must_use]
pub fn decode_polyline(encoded: &str) -> Option<Vec<(f64, f64)>> {
    let mut values = Vec::with_capacity(encoded.len() / 2);
    let mut bytes = encoded.bytes();
    while let Some(first) = bytes.next() {
        values.push(decode_value(first, &mut bytes)?);
    }
    if values.len() % 2 != 0 {
        return None;
    }
    let mut points = Vec::with_capacity(values.len() / 2);
    let (mut latitude, mut longitude) = (0_i64, 0_i64);
    let (pairs, _) = values.as_chunks::<2>();
    for &[latitude_delta, longitude_delta] in pairs {
        latitude = latitude.checked_add(latitude_delta)?;
        longitude = longitude.checked_add(longitude_delta)?;
        let point = (
            latitude as f64 / PRECISION_FACTOR,
            longitude as f64 / PRECISION_FACTOR,
        );
        if !(-90.0..=90.0).contains(&point.0) || !(-180.0..=180.0).contains(&point.1) {
            return None;
        }
        points.push(point);
    }
    Some(points)
}

/// Read one zig-zag encoded value whose first chunk is `first`.
fn decode_value(first: u8, rest: &mut impl Iterator<Item = u8>) -> Option<i64> {
    let mut accumulated = 0_i64;
    let mut shift = 0_u32;
    let mut byte = first;
    loop {
        let chunk = i64::from(chunk_payload(byte)?);
        accumulated |= (chunk & CHUNK_MASK) << shift;
        if chunk & CONTINUATION_BIT == 0 {
            break;
        }
        shift += CHUNK_BITS;
        if shift > MAX_SHIFT {
            return None;
        }
        byte = rest.next()?;
    }
    Some(if accumulated & 1 == 1 {
        !(accumulated >> 1)
    } else {
        accumulated >> 1
    })
}

/// The six bits a character carries, or `None` outside the alphabet.
fn chunk_payload(byte: u8) -> Option<u8> {
    byte.checked_sub(CHUNK_OFFSET)
        .filter(|payload| *payload < 64)
}

/// Encode `(latitude, longitude)` pairs in degrees as a polyline at precision 5.
///
/// Each coordinate is rounded to the nearest 1e-5 degree, so a route decoded
/// from a polyline re-encodes to the same string.
#[must_use]
pub fn encode_polyline(points: &[(f64, f64)]) -> String {
    let mut encoded = String::with_capacity(points.len() * 8);
    let (mut previous_latitude, mut previous_longitude) = (0_i64, 0_i64);
    for &(latitude, longitude) in points {
        let latitude = scaled(latitude);
        let longitude = scaled(longitude);
        encode_value(latitude - previous_latitude, &mut encoded);
        encode_value(longitude - previous_longitude, &mut encoded);
        previous_latitude = latitude;
        previous_longitude = longitude;
    }
    encoded
}

/// A coordinate in degrees as the integer the encoding carries.
fn scaled(degrees: f64) -> i64 {
    // A coordinate is at most 180 degrees, so the scaled value is at most
    // 18 000 000 and always fits.
    (degrees * PRECISION_FACTOR).round() as i64
}

/// Append one zig-zag encoded value to `out`.
fn encode_value(value: i64, out: &mut String) {
    let mut remaining = if value < 0 { !(value << 1) } else { value << 1 };
    while remaining >= CONTINUATION_BIT {
        out.push(chunk_char((remaining & CHUNK_MASK) | CONTINUATION_BIT));
        remaining >>= CHUNK_BITS;
    }
    out.push(chunk_char(remaining));
}

/// The character carrying a six-bit chunk.
fn chunk_char(chunk: i64) -> char {
    // Every caller passes a value below 64, so the sum stays in printable ASCII.
    char::from(chunk as u8 + CHUNK_OFFSET)
}
