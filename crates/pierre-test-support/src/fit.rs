// ABOUTME: Writes small, valid FIT activity files for tests — a file_id, timed records, one lap, one session
// ABOUTME: Encoded field by field from the FIT profile, with the header and data CRCs a decoder checks

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! FIT files for tests.
//!
//! A real device file pins the decoder against what a watch writes; these pin
//! everything around it — a workout at a start the test chooses (inside the
//! activity cache's retention window, or at the same minute as a provider's
//! copy), a file of another kind, a broken checksum. Each message is written
//! with its definition from the FIT profile (Garmin's FIT SDK, Profile.xlsx):
//! global message numbers, field numbers, base types and scales.

use chrono::{DateTime, Utc};

/// Seconds between the Unix epoch and FIT's (1989-12-31T00:00:00Z).
const FIT_EPOCH_OFFSET: i64 = 631_065_600;

/// Semicircles per degree: `2^31` per 180 degrees.
const SEMICIRCLES_PER_DEGREE: f64 = 2_147_483_648.0 / 180.0;

/// The sport a test workout is recorded as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitSport {
    /// FIT sport 1.
    Running,
    /// FIT sport 2.
    Cycling,
}

impl FitSport {
    const fn code(self) -> u8 {
        match self {
            Self::Running => 1,
            Self::Cycling => 2,
        }
    }

    /// Metres per second the samples move at.
    const fn speed(self) -> f64 {
        match self {
            Self::Running => 3.0,
            Self::Cycling => 8.0,
        }
    }
}

/// A workout to write as a FIT activity file.
#[derive(Debug, Clone, Copy)]
pub struct FitWorkout {
    /// When the first sample was recorded.
    pub start: DateTime<Utc>,
    /// How many one-second samples it holds.
    pub seconds: u32,
    /// The sport its session names.
    pub sport: FitSport,
    /// Whether its samples carry a position.
    pub with_position: bool,
}

impl FitWorkout {
    /// A ride of `seconds` one-second samples with a position, from `start`.
    #[must_use]
    pub const fn ride(start: DateTime<Utc>, seconds: u32) -> Self {
        Self {
            start,
            seconds,
            sport: FitSport::Cycling,
            with_position: true,
        }
    }

    /// The distance it covers, in metres.
    #[must_use]
    pub fn distance_meters(&self) -> f64 {
        self.sport.speed() * f64::from(self.seconds.saturating_sub(1))
    }

    /// The FIT activity file holding it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();
        file_id(&mut data, 4, self.start);
        records(&mut data, self);
        lap(&mut data, self);
        session(&mut data, self);
        activity(&mut data, self);
        fit_file(&data)
    }
}

/// A FIT course file: valid, but not a completed activity.
#[must_use]
pub fn course_file(created: DateTime<Utc>) -> Vec<u8> {
    let mut data = Vec::new();
    file_id(&mut data, 6, created);
    fit_file(&data)
}

/// FIT's CRC-16, nibble by nibble, as the FIT SDK specifies it.
#[must_use]
pub fn fit_crc(bytes: &[u8]) -> u16 {
    const TABLE: [u16; 16] = [
        0x0000, 0xCC01, 0xD801, 0x1400, 0xF001, 0x3C00, 0x2800, 0xE401, 0xA001, 0x6C00, 0x7800,
        0xB401, 0x5000, 0x9C01, 0x8801, 0x4400,
    ];
    bytes.iter().fold(0u16, |crc, byte| {
        let nibble = |crc: u16, n: u8| {
            let tmp = TABLE[usize::from(crc & 0xF)];
            let crc = (crc >> 4) & 0x0FFF;
            crc ^ tmp ^ TABLE[usize::from(n & 0xF)]
        };
        nibble(nibble(crc, *byte), *byte >> 4)
    })
}

/// The 14-byte header, the data records and the closing CRC.
fn fit_file(data: &[u8]) -> Vec<u8> {
    let mut file = vec![14, 0x20];
    file.extend_from_slice(&2132_u16.to_le_bytes());
    file.extend_from_slice(&u32::try_from(data.len()).unwrap_or(u32::MAX).to_le_bytes());
    file.extend_from_slice(b".FIT");
    let header_crc = fit_crc(&file);
    file.extend_from_slice(&header_crc.to_le_bytes());
    file.extend_from_slice(data);
    let crc = fit_crc(&file);
    file.extend_from_slice(&crc.to_le_bytes());
    file
}

/// FIT base types used below.
const ENUM: u8 = 0x00;
const UINT8: u8 = 0x02;
const UINT16: u8 = 0x84;
const SINT32: u8 = 0x85;
const UINT32: u8 = 0x86;

/// A definition message for local type `local`: global message `global` with
/// `(field number, size, base type)` fields, little endian.
fn define(data: &mut Vec<u8>, local: u8, global: u16, fields: &[(u8, u8, u8)]) {
    data.push(0x40 | local);
    data.push(0);
    data.push(0);
    data.extend_from_slice(&global.to_le_bytes());
    data.push(u8::try_from(fields.len()).unwrap_or(u8::MAX));
    for (number, size, base) in fields {
        data.extend_from_slice(&[*number, *size, *base]);
    }
}

fn fit_time(at: DateTime<Utc>) -> u32 {
    u32::try_from(at.timestamp() - FIT_EPOCH_OFFSET).unwrap_or(0)
}

#[allow(clippy::cast_possible_truncation)]
fn semicircles(degrees: f64) -> i32 {
    (degrees * SEMICIRCLES_PER_DEGREE).round() as i32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled_u32(value: f64, scale: f64) -> u32 {
    (value * scale).round() as u32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn scaled_u16(value: f64, scale: f64, offset: f64) -> u16 {
    ((value + offset) * scale).round() as u16
}

/// Where the sample at `second` sits: north of Montréal's Mont Royal,
/// moving north.
fn position(second: u32) -> (f64, f64) {
    (f64::from(second).mul_add(0.000_07, 45.5), -73.6)
}

fn heart_rate(second: u32) -> u8 {
    120 + u8::try_from(second % 20).unwrap_or(0)
}

/// `file_id`: type, manufacturer (255, development), time created.
fn file_id(data: &mut Vec<u8>, kind: u8, created: DateTime<Utc>) {
    define(data, 0, 0, &[(0, 1, ENUM), (1, 2, UINT16), (4, 4, UINT32)]);
    data.push(0);
    data.push(kind);
    data.extend_from_slice(&255_u16.to_le_bytes());
    data.extend_from_slice(&fit_time(created).to_le_bytes());
}

/// `record`: timestamp, position, altitude, heart rate, distance, speed.
fn records(data: &mut Vec<u8>, workout: &FitWorkout) {
    let mut fields = vec![(253, 4, UINT32)];
    if workout.with_position {
        fields.extend([(0, 4, SINT32), (1, 4, SINT32)]);
    }
    fields.extend([
        (2, 2, UINT16),
        (3, 1, UINT8),
        (5, 4, UINT32),
        (6, 2, UINT16),
    ]);
    define(data, 1, 20, &fields);
    let speed = workout.sport.speed();
    for second in 0..workout.seconds {
        data.push(1);
        let at = workout.start + chrono::Duration::seconds(i64::from(second));
        data.extend_from_slice(&fit_time(at).to_le_bytes());
        if workout.with_position {
            let (lat, long) = position(second);
            data.extend_from_slice(&semicircles(lat).to_le_bytes());
            data.extend_from_slice(&semicircles(long).to_le_bytes());
        }
        // altitude: scale 5, offset 500
        data.extend_from_slice(&scaled_u16(100.0, 5.0, 500.0).to_le_bytes());
        data.push(heart_rate(second));
        data.extend_from_slice(&scaled_u32(speed * f64::from(second), 100.0).to_le_bytes());
        data.extend_from_slice(&scaled_u16(speed, 1000.0, 0.0).to_le_bytes());
    }
}

/// The figures the lap and the session both state.
fn totals(workout: &FitWorkout) -> (u32, u32, u8, u8) {
    let elapsed = scaled_u32(f64::from(workout.seconds), 1000.0);
    let distance = scaled_u32(workout.distance_meters(), 100.0);
    let max = (0..workout.seconds).map(heart_rate).max().unwrap_or(0);
    let sum: u32 = (0..workout.seconds).map(|s| u32::from(heart_rate(s))).sum();
    let avg = u8::try_from(sum / workout.seconds.max(1)).unwrap_or(u8::MAX);
    (elapsed, distance, avg, max)
}

fn end_time(workout: &FitWorkout) -> u32 {
    fit_time(workout.start) + workout.seconds.saturating_sub(1)
}

/// `lap`: one lap over the whole workout.
fn lap(data: &mut Vec<u8>, workout: &FitWorkout) {
    define(
        data,
        2,
        19,
        &[
            (253, 4, UINT32),
            (2, 4, UINT32),
            (7, 4, UINT32),
            (8, 4, UINT32),
            (9, 4, UINT32),
            (15, 1, UINT8),
            (16, 1, UINT8),
            (25, 1, ENUM),
        ],
    );
    let (elapsed, distance, avg, max) = totals(workout);
    data.push(2);
    data.extend_from_slice(&end_time(workout).to_le_bytes());
    data.extend_from_slice(&fit_time(workout.start).to_le_bytes());
    data.extend_from_slice(&elapsed.to_le_bytes());
    data.extend_from_slice(&elapsed.to_le_bytes());
    data.extend_from_slice(&distance.to_le_bytes());
    data.push(avg);
    data.push(max);
    data.push(workout.sport.code());
}

/// `session`: the workout's summary.
fn session(data: &mut Vec<u8>, workout: &FitWorkout) {
    let mut fields = vec![(253, 4, UINT32), (2, 4, UINT32)];
    if workout.with_position {
        fields.extend([(3, 4, SINT32), (4, 4, SINT32)]);
    }
    fields.extend([
        (5, 1, ENUM),
        (6, 1, ENUM),
        (7, 4, UINT32),
        (8, 4, UINT32),
        (9, 4, UINT32),
        (16, 1, UINT8),
        (17, 1, UINT8),
        (26, 2, UINT16),
    ]);
    define(data, 3, 18, &fields);
    let (elapsed, distance, avg, max) = totals(workout);
    data.push(3);
    data.extend_from_slice(&end_time(workout).to_le_bytes());
    data.extend_from_slice(&fit_time(workout.start).to_le_bytes());
    if workout.with_position {
        let (lat, long) = position(0);
        data.extend_from_slice(&semicircles(lat).to_le_bytes());
        data.extend_from_slice(&semicircles(long).to_le_bytes());
    }
    data.push(workout.sport.code());
    data.push(0);
    data.extend_from_slice(&elapsed.to_le_bytes());
    data.extend_from_slice(&elapsed.to_le_bytes());
    data.extend_from_slice(&distance.to_le_bytes());
    data.push(avg);
    data.push(max);
    data.extend_from_slice(&1_u16.to_le_bytes());
}

/// `activity`: one session.
fn activity(data: &mut Vec<u8>, workout: &FitWorkout) {
    define(data, 4, 34, &[(253, 4, UINT32), (1, 2, UINT16)]);
    data.push(4);
    data.extend_from_slice(&end_time(workout).to_le_bytes());
    data.extend_from_slice(&1_u16.to_le_bytes());
}
