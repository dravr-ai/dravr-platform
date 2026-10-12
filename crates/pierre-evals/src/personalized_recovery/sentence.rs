// ABOUTME: Claim reader for the overnight-recovery probes — splits a claim into words, numbers and percent signs
// ABOUTME: Reads scopes, directions, units and durations near a metric keyword, in the five shipped locales
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::vocab::{
    AVERAGE, AVERAGING_CLAIMS, BPM_WORDS, CHANGE_WORDS, COMPARED_WORDS, CONNECTIVES, COUNTED_BACK,
    COUNT_BACK, DAY_WORDS, DIFFERENCE_WORDS, DOWN_PHRASES, DOWN_WORDS, EARLIER_NIGHT_PHRASES,
    FILLERS, HALF_HOUR, HOUR_WORDS, HRV_KEYWORDS, LAST_NIGHT, LINK_WORDS, LONGER_PERIODS,
    LONGER_PERIOD_PHRASES, MILLIS_WORDS, MINUTE_WORDS, OTHER_UNIT_WORDS, PAST_CHANGE,
    PERCENT_WORDS, PERIOD_WORDS, RESTING_HR_KEYWORDS, SECOND_PERSON, SLEEP_CLAIMS, SLEEP_NOUNS,
    TO_WORDS, UP_PHRASES, UP_WORDS, WEEK, WEEKDAYS,
};
use super::{BASELINE_DAYS, WINDOW_TOKENS};

/// A token range `[start, end)` in a [`Sentence`].
pub(super) type Span = (usize, usize);

/// Sort key for the figure nearest a keyword: token distance first, then a
/// figure after the keyword before one ahead of it.
pub(super) fn nearness(i: usize, anchor: Span) -> (usize, bool) {
    (distance(i, anchor), i < anchor.0)
}

/// Token distance from index `i` to a span.
pub(super) fn distance(i: usize, (start, end): Span) -> usize {
    if i < start {
        start - i
    } else {
        (i + 1).saturating_sub(end)
    }
}

/// Which reading a figure is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    /// The figure sits by "last night", "this morning" and the like.
    LastNight,
    /// The figure sits by an average, a mean or a baseline.
    Average,
    /// The figure sits by a week or by "nights": a mean, or one of the
    /// week's nights ("it dipped to 42 ms this week").
    Week,
    /// Nothing says which: the figure may be last night, a mean or one of the
    /// week's nights.
    Either,
}

/// Which way a stated change goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Direction {
    Down,
    Up,
}

/// The unit that follows a figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unit {
    Percent,
    Millis,
    Bpm,
    Hours,
    Minutes,
    /// A count of days, nights or weeks: never a reading.
    Period,
    /// Some other unit (km, watts, times): never one of these readings.
    Other,
    /// No unit: the next token is an ordinary word, punctuation or the end.
    Bare,
}

/// One token of a lowercased claim.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Number(f64),
    Percent,
}

/// A claim split into words, numbers and percent signs.
///
/// Words are runs of letters, so an apostrophe or a hyphen splits them
/// ("you've" is `you` `ve`, "7-day" is `7` `day`). A decimal comma is a
/// decimal point: "6,5 Stunden" is 6.5 hours.
pub(super) struct Sentence<'a> {
    tokens: Vec<Token<'a>>,
}

impl<'a> Sentence<'a> {
    pub(super) fn read(lower: &'a str) -> Self {
        let chars: Vec<(usize, char)> = lower.char_indices().collect();
        let byte_at = |k: usize| chars.get(k).map_or(lower.len(), |&(b, _)| b);
        let digit_at = |k: usize| chars.get(k).is_some_and(|&(_, c)| c.is_ascii_digit());
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let ch = chars[i].1;
            if ch.is_alphabetic() {
                let start = i;
                while chars.get(i).is_some_and(|&(_, c)| c.is_alphabetic()) {
                    i += 1;
                }
                tokens.push(Token::Word(&lower[byte_at(start)..byte_at(i)]));
            } else if ch.is_ascii_digit() || (is_sign(ch, i, &chars) && digit_at(i + 1)) {
                let mut number = String::new();
                if !ch.is_ascii_digit() {
                    number.push('-');
                    i += 1;
                }
                while digit_at(i) {
                    number.push(chars[i].1);
                    i += 1;
                }
                // A clock time or a pace ("6:30", "5:00/km") is never a
                // reading: it is skipped whole.
                if chars.get(i).is_some_and(|&(_, c)| c == ':') && digit_at(i + 1) {
                    while chars
                        .get(i)
                        .is_some_and(|&(_, c)| c == ':' || c.is_ascii_digit())
                    {
                        i += 1;
                    }
                    continue;
                }
                if chars.get(i).is_some_and(|&(_, c)| c == '.' || c == ',') && digit_at(i + 1) {
                    number.push('.');
                    i += 1;
                    while digit_at(i) {
                        number.push(chars[i].1);
                        i += 1;
                    }
                }
                if let Ok(value) = number.parse() {
                    tokens.push(Token::Number(value));
                }
            } else {
                if ch == '%' {
                    tokens.push(Token::Percent);
                }
                i += 1;
            }
        }
        Self { tokens }
    }

    pub(super) fn word(&self, i: usize) -> Option<&'a str> {
        match self.tokens.get(i) {
            Some(Token::Word(w)) => Some(w),
            _ => None,
        }
    }

    pub(super) fn number(&self, i: usize) -> Option<f64> {
        match self.tokens.get(i) {
            Some(Token::Number(v)) => Some(*v),
            _ => None,
        }
    }

    /// Every span where `phrase` (space-separated words) occurs.
    fn phrase_spans(&self, phrase: &str) -> Vec<Span> {
        let words: Vec<&str> = phrase.split_whitespace().collect();
        if words.is_empty() {
            return Vec::new();
        }
        self.tokens
            .windows(words.len())
            .enumerate()
            .filter(|(_, window)| {
                window
                    .iter()
                    .zip(&words)
                    .all(|(token, word)| *token == Token::Word(word))
            })
            .map(|(i, _)| (i, i + words.len()))
            .collect()
    }

    /// The earliest span where any of `phrases` occurs.
    fn find_phrase(&self, phrases: &[&str]) -> Option<Span> {
        phrases
            .iter()
            .flat_map(|p| self.phrase_spans(p))
            .min_by_key(|&(start, _)| start)
    }

    pub(super) fn has_phrase(&self, phrases: &[&str]) -> bool {
        self.find_phrase(phrases).is_some()
    }

    pub(super) fn has_word(&self, words: &[&str]) -> bool {
        self.tokens
            .iter()
            .any(|t| matches!(t, Token::Word(w) if words.contains(w)))
    }

    /// Addressed to the athlete, or reporting a change that happened: what
    /// separates a statement about the record from a generality.
    pub(super) fn is_about_the_athlete(&self) -> bool {
        self.has_word(SECOND_PERSON) || self.has_word(PAST_CHANGE)
    }

    /// Whether the sentence names a span longer than the week the readings
    /// cover — a month, several weeks, a year, more than seven days or
    /// nights — which a figure in it may be about.
    pub(super) fn names_a_longer_window(&self) -> bool {
        self.has_word(LONGER_PERIODS)
            || self.has_phrase(LONGER_PERIOD_PHRASES)
            || (0..self.tokens.len()).any(|i| {
                self.number(i).is_some_and(|n| n > BASELINE_DAYS as f64)
                    && self.word(i + 1).is_some_and(|w| DAY_WORDS.contains(&w))
            })
    }

    /// Whether the sentence names a night before the most recent one — a
    /// weekday, "the night before last", "three days ago" — which a figure
    /// in it may be about while last night and the means stand for another.
    pub(super) fn names_an_earlier_night(&self) -> bool {
        self.has_word(WEEKDAYS)
            || self.has_phrase(EARLIER_NIGHT_PHRASES)
            || COUNT_BACK
                .iter()
                .flat_map(|p| self.phrase_spans(p))
                .any(|(_, end)| {
                    (end..end + 2).any(|k| self.word(k).is_some_and(|w| COUNTED_BACK.contains(&w)))
                })
    }

    /// Indices of the numbers within [`WINDOW_TOKENS`] of `anchor` and no
    /// nearer another metric's keyword: in "your HRV dropped 15% while your
    /// resting HR held at 52", the 15% is the HRV's.
    pub(super) fn figures_near(&self, anchor: Span) -> impl Iterator<Item = usize> + '_ {
        let rivals: Vec<Span> = [
            hrv_anchor(self),
            resting_hr_anchor(self),
            sleep_anchor(self),
        ]
        .into_iter()
        .flatten()
        .filter(|rival| *rival != anchor)
        .collect();
        (0..self.tokens.len()).filter(move |&i| {
            let own = distance(i, anchor);
            self.number(i).is_some()
                && own <= WINDOW_TOKENS
                && rivals.iter().all(|&rival| own <= distance(i, rival))
        })
    }

    /// The unit following the number at `i`.
    pub(super) fn unit_after(&self, i: usize) -> Unit {
        match self.tokens.get(i + 1) {
            Some(Token::Percent) => Unit::Percent,
            Some(Token::Word(w)) => {
                let then = self.word(i + 2);
                let two_word_percent = matches!(
                    (*w, then),
                    ("pour" | "per", Some("cent")) | ("por", Some("ciento" | "cento"))
                );
                if two_word_percent || PERCENT_WORDS.contains(w) {
                    Unit::Percent
                } else if MILLIS_WORDS.contains(w) {
                    Unit::Millis
                } else if BPM_WORDS.contains(w) {
                    Unit::Bpm
                } else if HOUR_WORDS.contains(w) {
                    Unit::Hours
                } else if MINUTE_WORDS.contains(w) {
                    Unit::Minutes
                } else if PERIOD_WORDS.contains(w) {
                    Unit::Period
                } else if OTHER_UNIT_WORDS.contains(w) {
                    Unit::Other
                } else {
                    Unit::Bare
                }
            }
            _ => Unit::Bare,
        }
    }

    /// A sleep duration starting at the number at `i`, in hours, with the
    /// index just past it: `7 h`, `7h30`, `7 hours and 30 minutes`, `6,5
    /// Stunden`, `6 horas y media`, `450 minutes`.
    pub(super) fn duration_at(&self, i: usize) -> Option<(f64, usize)> {
        let value = self.number(i)?;
        // The "30" of "7 h 30" is part of the figure before it, not one of its own.
        let word_before = |back: usize| i.checked_sub(back).and_then(|k| self.word(k));
        if word_before(1).is_some_and(|w| HOUR_WORDS.contains(&w))
            || (word_before(1).is_some_and(|w| CONNECTIVES.contains(&w))
                && word_before(2).is_some_and(|w| HOUR_WORDS.contains(&w)))
        {
            return None;
        }
        match self.unit_after(i) {
            Unit::Minutes => Some((value / 60.0, i + 2)),
            Unit::Hours => {
                let (extra, end) = self.minutes_after_hours(i + 2);
                Some((value + extra, end))
            }
            _ => None,
        }
    }

    /// The minutes that follow an hour figure ending at `j`, in hours, with
    /// the index just past them.
    fn minutes_after_hours(&self, j: usize) -> (f64, usize) {
        let minutes_at = |k: usize| {
            self.number(k)
                .filter(|m| (0.0..60.0).contains(m) && m.fract().abs() < f64::EPSILON)
        };
        if let Some(m) = minutes_at(j) {
            match self.unit_after(j) {
                Unit::Minutes => return (m / 60.0, j + 2),
                Unit::Bare => return (m / 60.0, j + 1),
                _ => {}
            }
        }
        if self.word(j).is_some_and(|w| CONNECTIVES.contains(&w)) {
            if let Some(m) = minutes_at(j + 1) {
                if self.unit_after(j + 1) == Unit::Minutes {
                    return (m / 60.0, j + 3);
                }
            }
        }
        for half in HALF_HOUR {
            let words: Vec<&str> = half.split_whitespace().collect();
            if words
                .iter()
                .enumerate()
                .all(|(k, w)| self.word(j + k) == Some(w))
            {
                return (0.5, j + words.len());
            }
        }
        (0.0, j)
    }

    /// Whether the number at `i` (its figure ending at `end`) states a
    /// reading rather than a difference or a bound.
    ///
    /// A difference names a change before it ("dropped 5 ms", "by 5", "a
    /// baissé de 5") or a comparison after it ("5 bpm below", "2 hours more");
    /// a bound reads "more than 8 hours". A bare number is a reading only
    /// where a linking word puts it ("is 62", "liegt bei 48", "à 45") or right
    /// after its keyword ("HRV: 62").
    pub(super) fn is_reading_position(
        &self,
        i: usize,
        end: usize,
        anchor: Span,
        explicit_unit: bool,
    ) -> bool {
        let prev = i.checked_sub(1).and_then(|p| self.word(p));
        let prev2 = i.checked_sub(2).and_then(|p| self.word(p));
        let changes = |w: &str| CHANGE_WORDS.contains(&w) || PAST_CHANGE.contains(&w);
        let mut after = self.word(end);
        if after.is_some_and(|w| FILLERS.contains(&w)) {
            after = self.word(end + 1);
        }
        let difference = prev.is_some_and(|w| changes(w) || DIFFERENCE_WORDS.contains(&w))
            || (prev.is_some_and(|w| LINK_WORDS.contains(&w)) && prev2.is_some_and(changes))
            || after.is_some_and(|w| COMPARED_WORDS.contains(&w));
        if difference {
            return false;
        }
        explicit_unit
            || i == anchor.1
            || prev.is_some_and(|w| LINK_WORDS.contains(&w) || TO_WORDS.contains(&w))
    }

    /// The scope of the figure spanning `figure`: the nearest "last night",
    /// average or week phrase with no other number between them, or
    /// [`Scope::Either`]. An average and a week equally near read as the
    /// week, the scope that contradicts less.
    pub(super) fn scope_of(&self, (start, end): Span) -> Scope {
        let nearest = |phrases: &[&str]| {
            phrases
                .iter()
                .flat_map(|p| self.phrase_spans(p))
                .filter_map(|span| {
                    let (between, gap) = if span.0 >= end {
                        (end..span.0, span.0 + 1 - end)
                    } else {
                        (span.1..start, (start + 1).saturating_sub(span.1))
                    };
                    (between.into_iter().all(|k| self.number(k).is_none()) && gap <= WINDOW_TOKENS)
                        .then_some(gap)
                })
                .min()
        };
        let period = match (nearest(AVERAGE), nearest(WEEK)) {
            (Some(avg), Some(week)) if avg < week => Some((avg, Scope::Average)),
            (Some(avg), None) => Some((avg, Scope::Average)),
            (_, Some(week)) => Some((week, Scope::Week)),
            (None, None) => None,
        };
        match (nearest(LAST_NIGHT), period) {
            (Some(last), Some((gap, _))) if last < gap => Scope::LastNight,
            (Some(last), Some((gap, scope))) if gap < last => scope,
            (Some(_), None) => Scope::LastNight,
            (None, Some((_, scope))) => scope,
            _ => Scope::Either,
        }
    }

    /// The direction the sentence gives a change, when it names exactly one.
    pub(super) fn direction(&self) -> Option<Direction> {
        let down = self.has_word(DOWN_WORDS) || self.has_phrase(DOWN_PHRASES);
        let up = self.has_word(UP_WORDS) || self.has_phrase(UP_PHRASES);
        match (down, up) {
            (true, false) => Some(Direction::Down),
            (false, true) => Some(Direction::Up),
            _ => None,
        }
    }
}

/// Where a sentence names HRV.
pub(super) fn hrv_anchor(sentence: &Sentence<'_>) -> Option<Span> {
    sentence.find_phrase(HRV_KEYWORDS)
}

/// Where a sentence names resting heart rate.
pub(super) fn resting_hr_anchor(sentence: &Sentence<'_>) -> Option<Span> {
    sentence.find_phrase(RESTING_HR_KEYWORDS)
}

/// Where a sentence states something about the athlete's sleep.
pub(super) fn sleep_anchor(sentence: &Sentence<'_>) -> Option<Span> {
    sentence.find_phrase(SLEEP_CLAIMS).or_else(|| {
        sentence
            .has_phrase(SLEEP_NOUNS)
            .then(|| sentence.find_phrase(AVERAGING_CLAIMS))
            .flatten()
    })
}

/// A minus sign opening a number: at the start, or after a non-alphanumeric.
fn is_sign(ch: char, i: usize, chars: &[(usize, char)]) -> bool {
    matches!(ch, '-' | '\u{2212}')
        && i.checked_sub(1)
            .and_then(|p| chars.get(p))
            .is_none_or(|&(_, c)| !c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_durations_parse_in_their_written_forms() {
        for (text, hours) in [
            ("7h30", 7.5),
            ("7 h 30", 7.5),
            ("7 hours and 30 minutes", 7.5),
            ("6,5 stunden", 6.5),
            ("6 horas y media", 6.5),
            ("7 heures et demie", 7.5),
            ("450 minutes", 7.5),
            ("8 hrs", 8.0),
        ] {
            let s = Sentence::read(text);
            assert_eq!(s.duration_at(0).map(|(h, _)| h), Some(hours), "{text:?}");
        }
        // A count of nights is not a duration.
        assert_eq!(Sentence::read("3 nights").duration_at(0), None);
        // Nor are the minutes of an hour figure one of their own.
        assert_eq!(Sentence::read("7 h 30 min").duration_at(2), None);
        assert_eq!(
            Sentence::read("7 hours and 30 minutes").duration_at(3),
            None
        );
    }
}
