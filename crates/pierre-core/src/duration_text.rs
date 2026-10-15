// ABOUTME: An activity duration as the model reads it — H:MM:SS, exact to the second
// ABOUTME: One format for every LLM-facing rendering, so no surface rounds a figure the coach quotes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// `seconds` as a clock duration, always `H:MM:SS`.
///
/// Exact to the second, because the model quotes these figures and derives
/// pace from them. A rounded duration becomes a wrong figure in a reply: a
/// 53-minute run rendered as `0.9h` read back as 54 minutes and a pace
/// 10 s/km slow, and a coach challenged on it «corrected» a right answer to
/// that (carnet#1015).
///
/// Always with the hour, because the model reads these beside start times:
/// an activity row carries `22:59` for when it started, and a bare `53:00`
/// beside it, or `6:00 active` on a roster line, reads as hours. `0:53:00`
/// cannot. A race time shown to the athlete (a personal-best notification)
/// is a different reader and keeps its own `m:ss` convention.
#[must_use]
pub fn clock(seconds: u64) -> String {
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let secs = seconds % 60;
    format!("{hours}:{minutes:02}:{secs:02}")
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn a_duration_renders_to_the_second() {
        assert_eq!(clock(3_180), "0:53:00");
        assert_eq!(clock(3_239), "0:53:59");
        assert_eq!(clock(4_200), "1:10:00");
        assert_eq!(clock(36_005), "10:00:05");
        assert_eq!(clock(360), "0:06:00");
        assert_eq!(clock(0), "0:00:00");
    }
}
