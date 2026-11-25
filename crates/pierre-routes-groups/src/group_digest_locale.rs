// ABOUTME: The language a group's chat reads its weekly digest in: the one most of its members read
// ABOUTME: A pure count of the members' locales, the owner's breaking a tie, testable without a chat
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The weekly digest's room language: every member votes with the locale
//! they read on the group's channel, and the most-read one wins
//! ([`plurality_locale`]).

/// The language most of `votes` name; among equals the owner's when it is one
/// of them, else the first one voted. With no votes, the owner's.
#[must_use]
pub fn plurality_locale<'a>(votes: impl IntoIterator<Item = &'a str>, owner: &str) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for vote in votes {
        match counts.iter_mut().find(|(locale, _)| *locale == vote) {
            Some((_, count)) => *count += 1,
            None => counts.push((vote, 1)),
        }
    }
    let top = counts.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let leaders: Vec<&str> = counts
        .iter()
        .filter(|(_, count)| *count == top)
        .map(|(locale, _)| *locale)
        .collect();
    leaders
        .iter()
        .find(|locale| **locale == owner)
        .or_else(|| leaders.first())
        .map_or(owner, |locale| *locale)
        .to_owned()
}
