// ABOUTME: Wahoo workout FIT file — the activity file Wahoo's CDN serves, decoded into a per-sample time series
// ABOUTME: Power, heart rate, cadence, speed, altitude, temperature and distance; never the GPS track (restriction iii)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Wahoo workout FIT files
//!
//! A completed Wahoo workout's summary names an *activity* FIT file on
//! Wahoo's CDN (`workout_summary.file.url`). Downloading it is not an API
//! call: it spends no request budget and does not revoke a previous access
//! token (Wahoo Cloud API, "Token Limits").
//!
//! The decoded series carries every channel the record messages hold except
//! position. Restriction (iii) of the Wahoo API Agreement bars using the API
//! "to aggregate, cache, or store geographic location information", and a
//! time series is cached with its activity, so the track is never read out of
//! the file — the same rule the routes scope follows (legal read, Part 8).

use reqwest::Url;

use crate::constants::oauth_providers;
use crate::errors::{AppError, AppResult};
use crate::fit_file::{FitActivityFile, Position};
use crate::models::TimeSeriesData;

/// The largest activity file read: a multi-hour ride recorded every second
/// is a few megabytes, so anything past this is not a workout file.
pub const MAX_FIT_BYTES: usize = 32 * 1024 * 1024;

/// Whether `url` is one Dravr fetches a workout file from: HTTPS on Wahoo's
/// own hosts. The URL arrives in an API response; anything else is refused
/// rather than fetched on Wahoo's say-so.
///
/// The host is the one the HTTP client will connect to, read by the same URL
/// parser: a hand split of the text is fooled by input the parser reads
/// differently (`https://evil.example\@cdn.wahooligan.com/` names
/// `evil.example`).
#[must_use]
pub fn is_wahoo_file_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    // Lowercased by the parser; an IP address never ends in a domain below.
    let Some(host) = parsed.host_str() else {
        return false;
    };
    ["wahooligan.com", "wahoofitness.com"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// Decode an activity FIT file into the workout's time series.
///
/// Samples are the file's `record` messages, in file order, timed from the
/// first one, read by the shared decoder ([`FitActivityFile`]) with the
/// position withheld. A file with no record message is a workout that
/// recorded no samples.
///
/// # Errors
///
/// Returns an error when the bytes are not a FIT file or fail its checksum.
pub fn time_series_from_fit(bytes: &[u8]) -> AppResult<TimeSeriesData> {
    let file = FitActivityFile::decode(bytes).map_err(|e| {
        AppError::external_service(
            oauth_providers::WAHOO,
            format!("workout FIT file did not decode: {e}"),
        )
    })?;
    Ok(file.time_series(Position::Withheld))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FIT's CRC-16, nibble by nibble, as the FIT SDK specifies it.
    fn fit_crc(bytes: &[u8]) -> u16 {
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

    /// Seconds between the Unix epoch and FIT's (1989-12-31T00:00:00Z).
    const FIT_EPOCH_OFFSET: u32 = 631_065_600;

    /// A FIT file of `record` messages carrying a timestamp, heart rate,
    /// power and a position — the position must never reach the series.
    fn fit_file(samples: &[(u32, u8, u16)]) -> Vec<u8> {
        let mut data = vec![
            0x40, 0x00, 0x00, // definition, local type 0, reserved, little endian
            20, 0x00, // global message 20: record
            5,    // fields
            253, 4, 0x86, // timestamp: uint32
            3, 1, 0x02, // heart_rate: uint8
            7, 2, 0x84, // power: uint16
            0, 4, 0x85, // position_lat: sint32
            1, 4, 0x85, // position_long: sint32
        ];
        for (unix, heart_rate, power) in samples {
            data.push(0x00);
            data.extend_from_slice(&(unix - FIT_EPOCH_OFFSET).to_le_bytes());
            data.push(*heart_rate);
            data.extend_from_slice(&power.to_le_bytes());
            data.extend_from_slice(&536_870_912_i32.to_le_bytes());
            data.extend_from_slice(&(-536_870_912_i32).to_le_bytes());
        }
        let mut file = vec![14, 0x20];
        file.extend_from_slice(&2132_u16.to_le_bytes());
        file.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_le_bytes());
        file.extend_from_slice(b".FIT");
        let header_crc = fit_crc(&file);
        file.extend_from_slice(&header_crc.to_le_bytes());
        file.extend_from_slice(&data);
        let crc = fit_crc(&file);
        file.extend_from_slice(&crc.to_le_bytes());
        file
    }

    #[test]
    fn record_messages_become_samples_timed_from_the_first() {
        let start = 1_791_000_000;
        let file = fit_file(&[
            (start, 120, 180),
            (start + 1, 125, 210),
            (start + 3, 130, 0),
        ]);
        let series = time_series_from_fit(&file).unwrap_or_else(|e| panic!("decode: {e}"));
        assert_eq!(series.timestamps, vec![0, 1, 3]);
        assert_eq!(
            series.heart_rate,
            Some(vec![Some(120), Some(125), Some(130)])
        );
        assert_eq!(series.power, Some(vec![Some(180), Some(210), Some(0)]));
        assert_eq!(
            series.cadence, None,
            "a channel no sample carries is absent"
        );
        assert_eq!(series.distance, None);
    }

    #[test]
    fn the_gps_track_is_never_read_out_of_the_file() {
        let file = fit_file(&[(1_791_000_000, 120, 180)]);
        let series = time_series_from_fit(&file).unwrap_or_else(|e| panic!("decode: {e}"));
        assert_eq!(series.gps_coordinates, None);
    }

    #[test]
    fn a_file_without_records_recorded_no_samples() {
        let series = time_series_from_fit(&fit_file(&[])).unwrap_or_else(|e| panic!("{e}"));
        assert!(series.timestamps.is_empty());
    }

    #[test]
    fn bytes_that_are_not_a_fit_file_are_an_error() {
        assert!(time_series_from_fit(b"not a fit file").is_err());
    }

    #[test]
    fn only_https_urls_on_wahoos_hosts_are_fetched() {
        for url in [
            "https://cdn.wahooligan.com/wahoo-cloud/production/uploads/workout_file/x/ride.fit",
            "https://wahooligan.com/file.fit",
        ] {
            assert!(is_wahoo_file_url(url), "{url}");
        }
        for url in [
            "http://cdn.wahooligan.com/ride.fit",
            "https://cdn.wahooligan.com.evil.example/ride.fit",
            "https://evilwahooligan.com/ride.fit",
            // The client reads a backslash as a path separator, so this names
            // evil.example, whatever follows it.
            "https://evil.example\\@cdn.wahooligan.com/ride.fit",
            "https://cdn.wahooligan.com@evil.example/ride.fit",
            "https://169.254.169.254/latest/meta-data",
            "file:///etc/passwd",
        ] {
            assert!(!is_wahoo_file_url(url), "{url}");
        }
    }
}
