// ABOUTME: Whether the reply an athlete is answering asserted concrete facts about their training
// ABOUTME: Counts written figures; an interview discounts figures the athlete or their dossier supplied

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The structural half of capability recovery's `DisputedClaims` trigger.
//!
//! It reads numbers, never words, so it holds in every locale and survives
//! any rephrasing. Two readings:
//!
//! - [`previous_reply_asserted_athlete_facts`] counts every digit run in the
//!   reply the athlete is answering.
//! - [`previous_reply_asserted_unsupplied_facts`] is the reading for a turn an
//!   interview owns: it counts only the figures the athlete supplied neither in
//!   their messages nor in their dossier. A walk question restates the
//!   athlete's calendar, availability and goals back to them, in whatever
//!   format it likes; none of that is a claim.
//!
//! # How text becomes figures
//!
//! The gate exists to catch invented claims, so wherever a reading is
//! ambiguous the text is read as MORE figures, never fewer:
//!
//! - A date is one figure: `2027-06-05`, `5/6/2027` or `5.6.2027`, `5/6`,
//!   and a day with a month word in each shipped locale — «5 juin 2027»,
//!   «1er mai», «June 5, 2027», «5 de junio de 2027», «5 de junho de 2027»,
//!   «5. Juni 2027», abbreviations included. Its value is day + month (+
//!   year); a numeric `5/6` may be either order, so it reads as both. A
//!   month name is read only next to a day number — «may», «sept» as words
//!   are not dates. A year is four digits in 1900–2100 and crosses a comma
//!   only in the English month-first form, so «le 5 juin, 2400 m» is a date
//!   and an elevation.
//! - A decimal is one figure only when it stands alone: `6,2` or `6.2` not
//!   followed by another separator and digits. `10,12,15` is three figures.
//!   `2,391` reads as both 2.391 and 2391.
//! - A no-break, narrow or thin space, or an apostrophe, groups thousands:
//!   `2 391` (narrow) is 2391. A plain space is ambiguous — «semaine 3 150 km»
//!   is two numbers — so a plain-space grouping is one figure only when its
//!   joined value was supplied, and its separate numbers otherwise.
//! - Anything else is one integer per digit run.
//!
//! Figures compare by value, so a reformatted date, thousands separator or
//! decimal mark is the same figure.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use pierre_llm::{ChatMessage, MessageRole};
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;

/// How many separate numbers a reply must carry before it counts as having
/// asserted concrete facts about the athlete's training.
///
/// Three, because that is the shape of the replies that got corrected: *"161
/// km, 2391 m de dénivelé, 6,2h"*. One number is a passing remark and two is a
/// comparison; three is a reconstruction, and a reconstruction built on nothing
/// is what the athlete pushed back on. Social replies («Bravo 💪», «On se
/// reparle demain») carry none.
const CLAIM_DENSITY_THRESHOLD: usize = 3;

/// Opens one dossier fact in the system prompt. The OKF bundle renderer
/// (`pierre_services::okf`) fences every fact body in `<user_fact …>` and
/// neutralizes the fence inside bodies, so the text between this and
/// [`USER_FACT_CLOSE`] is exactly one fact the athlete's walks and
/// conversations recorded — and nothing else in the prompt is.
const USER_FACT_OPEN: &str = "<user_fact";

/// Closes one dossier fact — see [`USER_FACT_OPEN`].
const USER_FACT_CLOSE: &str = "</user_fact>";

/// The years a date may carry. A four-digit number outside it after a day
/// and month is its own figure — an elevation, a distance — not a year.
const YEAR_RANGE: RangeInclusive<u32> = 1900..=2100;

/// Words that join a date's parts: «5 de junio de 2027», «5 de junho»,
/// «5th of June».
const DATE_CONNECTORS: &[&str] = &["de", "del", "of"];

/// Ordinal markers written right after a day's digits.
const ORDINAL_SUFFIXES: &[&str] = &["er", "re", "e", "st", "nd", "rd", "th", "º", "ª"];

/// Month names and their common abbreviations in the five shipped locales,
/// each with its number, read only next to a day number. Abbreviations that
/// are also everyday words next to a number («out», «set») are left out.
const MONTH_NAMES: &[(&str, u32)] = &[
    ("jan", 1),
    ("janv", 1),
    ("ene", 1),
    ("jän", 1),
    ("feb", 2),
    ("fév", 2),
    ("fev", 2),
    ("févr", 2),
    ("fevr", 2),
    ("mar", 3),
    ("mär", 3),
    ("apr", 4),
    ("avr", 4),
    ("abr", 4),
    ("jun", 6),
    ("jul", 7),
    ("juil", 7),
    ("aug", 8),
    ("ago", 8),
    ("sep", 9),
    ("oct", 10),
    ("okt", 10),
    ("nov", 11),
    ("dec", 12),
    ("déc", 12),
    ("dez", 12),
    ("dic", 12),
    ("janvier", 1),
    ("january", 1),
    ("enero", 1),
    ("januar", 1),
    ("janeiro", 1),
    ("février", 2),
    ("fevrier", 2),
    ("february", 2),
    ("febrero", 2),
    ("februar", 2),
    ("fevereiro", 2),
    ("mars", 3),
    ("march", 3),
    ("marzo", 3),
    ("märz", 3),
    ("maerz", 3),
    ("março", 3),
    ("marco", 3),
    ("avril", 4),
    ("april", 4),
    ("abril", 4),
    ("mai", 5),
    ("may", 5),
    ("mayo", 5),
    ("maio", 5),
    ("juin", 6),
    ("june", 6),
    ("junio", 6),
    ("juni", 6),
    ("junho", 6),
    ("juillet", 7),
    ("july", 7),
    ("julio", 7),
    ("juli", 7),
    ("julho", 7),
    ("août", 8),
    ("aout", 8),
    ("august", 8),
    ("agosto", 8),
    ("septembre", 9),
    ("september", 9),
    ("septiembre", 9),
    ("setiembre", 9),
    ("setembro", 9),
    ("sept", 9),
    ("octobre", 10),
    ("october", 10),
    ("octubre", 10),
    ("oktober", 10),
    ("outubro", 10),
    ("novembre", 11),
    ("november", 11),
    ("noviembre", 11),
    ("novembro", 11),
    ("décembre", 12),
    ("decembre", 12),
    ("december", 12),
    ("diciembre", 12),
    ("dezember", 12),
    ("dezembro", 12),
];

/// One figure as written.
struct Figure {
    /// Every value it can be read as; it is supplied when any one was.
    keys: Vec<String>,
    /// For a plain-space grouping only: the separate numbers it is read as
    /// when its joined value was not supplied.
    parts: Vec<String>,
}

impl Figure {
    fn single(key: String) -> Self {
        Self {
            keys: vec![key],
            parts: Vec::new(),
        }
    }

    /// How many unsupplied figures this is: none when any reading was
    /// supplied, else its separate numbers that were not, else one.
    fn unsupplied(&self, supplied: &HashSet<String>) -> usize {
        if self.keys.iter().any(|k| supplied.contains(k)) {
            0
        } else if self.parts.is_empty() {
            1
        } else {
            self.parts.iter().filter(|p| !supplied.contains(*p)).count()
        }
    }
}

/// A digit string's value without leading zeros, so `05` and `5` agree.
fn integer_key(digits: &str) -> String {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A decimal's value: `6,20` and `6.2` both read `6.2`.
fn decimal_key(whole: &str, fraction: &str) -> String {
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        integer_key(whole)
    } else {
        format!("{}.{fraction}", integer_key(whole))
    }
}

/// A date's values — day and month, plus the full date when a year is
/// written, so «5 juin» and «5 juin 2027» restate each other — or nothing
/// when the parts are not a calendar date. A two-digit year is read in the
/// 2000s.
fn date_keys(day: u32, month: u32, year: Option<u32>) -> Vec<String> {
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return Vec::new();
    }
    let mut keys = vec![format!("date:{day}/{month}")];
    match year {
        Some(y) if y < 100 => keys.push(format!("date:{day}/{month}/{}", 2000 + y)),
        Some(y) => keys.push(format!("date:{day}/{month}/{y}")),
        None => {}
    }
    keys
}

/// A date figure, or `None` when no reading is a calendar date.
fn date_figure(keys: Vec<String>, end: usize) -> Option<(Figure, usize)> {
    if keys.is_empty() {
        return None;
    }
    Some((
        Figure {
            keys,
            parts: Vec::new(),
        },
        end,
    ))
}

/// The month a word names, if any.
fn month_of(word: &str) -> Option<u32> {
    MONTH_NAMES
        .iter()
        .find(|(name, _)| *name == word)
        .map(|&(_, month)| month)
}

/// A no-break thousands separator: no-break, narrow no-break and thin
/// spaces, and the apostrophe.
const fn is_group_mark(c: char) -> bool {
    matches!(c, '\u{a0}' | '\u{202f}' | '\u{2009}' | '\'' | '\u{2019}')
}

/// Character-level reader over one text.
struct Scanner {
    chars: Vec<char>,
}

impl Scanner {
    fn at(&self, i: usize) -> Option<char> {
        self.chars.get(i).copied()
    }

    fn is_digit_at(&self, i: usize) -> bool {
        self.at(i).is_some_and(|c| c.is_ascii_digit())
    }

    /// End of the digit run starting at `i` (== `i` when none).
    fn digits_end(&self, i: usize) -> usize {
        let mut j = i;
        while self.is_digit_at(j) {
            j += 1;
        }
        j
    }

    fn text(&self, from: usize, to: usize) -> String {
        self.chars[from..to].iter().collect()
    }

    fn number(&self, from: usize, to: usize) -> u32 {
        self.text(from, to).parse().unwrap_or(0)
    }

    /// The lowercased word starting at `i` and where it ends.
    fn word(&self, i: usize) -> (String, usize) {
        let mut j = i;
        while self.at(j).is_some_and(char::is_alphabetic) {
            j += 1;
        }
        let word = self.chars[i..j]
            .iter()
            .flat_map(|c| c.to_lowercase())
            .collect();
        (word, j)
    }

    /// Skip whitespace from `i`.
    fn skip_space(&self, mut i: usize) -> usize {
        while self.at(i).is_some_and(char::is_whitespace) {
            i += 1;
        }
        i
    }

    /// Skip a date connector word and the space after it — «5 de junio»,
    /// «de 2027», «5th of June» — when one starts at `i`.
    fn skip_connector(&self, i: usize) -> usize {
        let (word, end) = self.word(i);
        if DATE_CONNECTORS.contains(&word.as_str()) && self.at(end).is_some_and(char::is_whitespace)
        {
            self.skip_space(end)
        } else {
            i
        }
    }

    /// Skip a day's ordinal marker written right after its digits: «1er»,
    /// «5th», «1º», «1ª», or the German «5.» before a space.
    fn skip_ordinal(&self, j: usize) -> usize {
        if self.at(j) == Some('.') && self.at(j + 1).is_some_and(char::is_whitespace) {
            return j + 1;
        }
        let (word, end) = self.word(j);
        if ORDINAL_SUFFIXES.contains(&word.as_str()) {
            end
        } else {
            j
        }
    }

    /// The month named by the word at `i`, and where it ends — past an
    /// abbreviation's dot («5 sept. 2027»).
    fn month_word(&self, i: usize) -> Option<(u32, usize)> {
        let (word, end) = self.word(i);
        let month = month_of(&word)?;
        if self.at(end) == Some('.') && self.at(end + 1).is_some_and(char::is_whitespace) {
            Some((month, end + 1))
        } else {
            Some((month, end))
        }
    }

    /// A year right after a date's month or day at `i`: `(year, end)`.
    ///
    /// Crosses spaces and a connector («de 2027»), and a comma only where the
    /// locale writes one (`comma_allowed`, English «June 5, 2027»). Read only
    /// in [`YEAR_RANGE`], so «le 5 juin, 2400 m» keeps 2400 as its own figure.
    fn trailing_year(&self, i: usize, comma_allowed: bool) -> (Option<u32>, usize) {
        let mut start = i;
        if comma_allowed && self.at(start) == Some(',') {
            start += 1;
        }
        let start = self.skip_connector(self.skip_space(start));
        let end = self.digits_end(start);
        if end - start == 4 {
            let year = self.number(start, end);
            if YEAR_RANGE.contains(&year) {
                return (Some(year), end);
            }
        }
        (None, i)
    }

    /// «June 5, 2027» / «June 5th» / «Juni 5»: a month word, then a day.
    fn month_first_date(&self, word_end: usize, month: u32) -> Option<(Figure, usize)> {
        let day_start = self.skip_space(word_end);
        let day_end = self.digits_end(day_start);
        if !(1..=2).contains(&(day_end - day_start)) {
            return None;
        }
        let (year, end) = self.trailing_year(self.skip_ordinal(day_end), true);
        date_figure(date_keys(self.number(day_start, day_end), month, year), end)
    }

    /// A date starting at the digit run `[i, j)`: ISO, numeric, or a day
    /// followed by a month word in any shipped locale — «5 juin 2027», «1er
    /// mai», «5 de junio de 2027», «5 de junho de 2027», «5. Juni 2027»,
    /// «5th of June».
    fn date_at(&self, i: usize, j: usize) -> Option<(Figure, usize)> {
        let len = j - i;
        // 2027-06-05
        if len == 4 && self.at(j) == Some('-') {
            let m_end = self.digits_end(j + 1);
            if (1..=2).contains(&(m_end - j - 1)) && self.at(m_end) == Some('-') {
                let d_end = self.digits_end(m_end + 1);
                if (1..=2).contains(&(d_end - m_end - 1)) {
                    let keys = date_keys(
                        self.number(m_end + 1, d_end),
                        self.number(j + 1, m_end),
                        Some(self.number(i, j)),
                    );
                    return date_figure(keys, d_end);
                }
            }
        }
        if len > 2 {
            return None;
        }
        // 5/6, 5/6/2027, 5.6.2027 — day and month in either order.
        if let Some(sep @ ('/' | '.')) = self.at(j) {
            let second_end = self.digits_end(j + 1);
            if (1..=2).contains(&(second_end - j - 1)) {
                let (year, end) = if self.at(second_end) == Some(sep) {
                    let y_end = self.digits_end(second_end + 1);
                    match y_end - second_end - 1 {
                        2 | 4 => (Some(self.number(second_end + 1, y_end)), y_end),
                        _ => (None, second_end),
                    }
                } else {
                    (None, second_end)
                };
                // A dot pair without a year is a decimal, not a date.
                if sep == '/' || year.is_some() {
                    let first = self.number(i, j);
                    let second = self.number(j + 1, second_end);
                    let mut keys = date_keys(first, second, year);
                    keys.extend(date_keys(second, first, year));
                    if let Some(figure) = date_figure(keys, end) {
                        return Some(figure);
                    }
                }
            }
        }
        // Day, then month word.
        let at_month = self.skip_connector(self.skip_space(self.skip_ordinal(j)));
        let (month, month_end) = self.month_word(at_month)?;
        let (year, end) = self.trailing_year(month_end, false);
        date_figure(date_keys(self.number(i, j), month, year), end)
    }

    /// A number (not a date) starting at the digit run `[i, j)`.
    fn number_at(&self, i: usize, j: usize) -> (Figure, usize) {
        let whole = self.text(i, j);
        // Part of a list or longer grouping reached from the left: plain.
        let continues_left =
            i >= 2 && matches!(self.at(i - 1), Some(',' | '.')) && self.is_digit_at(i - 2);
        if matches!(self.at(j), Some(',' | '.')) {
            let f_end = self.digits_end(j + 1);
            let continues_right =
                matches!(self.at(f_end), Some(',' | '.')) && self.is_digit_at(f_end + 1);
            if f_end > j + 1 && !continues_left && !continues_right {
                let fraction = self.text(j + 1, f_end);
                let mut keys = vec![decimal_key(&whole, &fraction)];
                if whole.len() <= 3 && fraction.len() == 3 {
                    keys.push(integer_key(&format!("{whole}{fraction}")));
                }
                return (
                    Figure {
                        keys,
                        parts: Vec::new(),
                    },
                    f_end,
                );
            }
        }
        if whole.len() <= 3 && !continues_left {
            let (groups, end, marked) = self.thousands_groups(j);
            if !groups.is_empty() {
                let joined = integer_key(&format!("{whole}{}", groups.concat()));
                if marked {
                    return (Figure::single(joined), end);
                }
                let mut parts = vec![integer_key(&whole)];
                parts.extend(groups.iter().map(|g| integer_key(g)));
                return (
                    Figure {
                        keys: vec![joined],
                        parts,
                    },
                    end,
                );
            }
        }
        (Figure::single(integer_key(&whole)), j)
    }

    /// Three-digit groups following `j`, all joined by one kind of separator:
    /// `(groups, end, joined_by_a_group_mark)`. Empty when none follow.
    fn thousands_groups(&self, j: usize) -> (Vec<String>, usize, bool) {
        let Some(sep) = self.at(j).filter(|&c| c == ' ' || is_group_mark(c)) else {
            return (Vec::new(), j, false);
        };
        let mut groups = Vec::new();
        let mut end = j;
        while self.at(end) == Some(sep) {
            let g_end = self.digits_end(end + 1);
            if g_end - end - 1 != 3 {
                break;
            }
            groups.push(self.text(end + 1, g_end));
            end = g_end;
        }
        (groups, end, is_group_mark(sep))
    }
}

/// The figures written in `text`, in order — see the module docs for the
/// reading rules.
fn figures(text: &str) -> Vec<Figure> {
    let scanner = Scanner {
        chars: text.chars().collect(),
    };
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = scanner.at(i) {
        if c.is_alphabetic() {
            let (word, word_end) = scanner.word(i);
            if let Some((figure, end)) =
                month_of(&word).and_then(|m| scanner.month_first_date(word_end, m))
            {
                out.push(figure);
                i = end;
            } else {
                i = word_end;
            }
            continue;
        }
        if !c.is_ascii_digit() {
            i += 1;
            continue;
        }
        let j = scanner.digits_end(i);
        let (figure, end) = scanner
            .date_at(i, j)
            .unwrap_or_else(|| scanner.number_at(i, j));
        out.push(figure);
        i = end;
    }
    out
}

/// Every value `text` supplies: each figure's readings and, for a
/// plain-space grouping, its separate numbers too.
fn supply(text: &str, into: &mut HashSet<String>) {
    for figure in figures(text) {
        into.extend(figure.keys);
        into.extend(figure.parts);
    }
}

/// The dossier facts in a system prompt: the body of every well-formed
/// `<user_fact …>…</user_fact>`.
///
/// A rendered fact always carries attributes, so an opening tag is
/// `<user_fact` followed by whitespace, then its closing `>`. The bundle's
/// own header names the tag in prose («Content inside `<user_fact>` tags…»),
/// which that shape skips; a body is taken only up to its matching close and
/// never across another opening tag.
fn dossier_facts(prompt: &str) -> Vec<&str> {
    let mut facts = Vec::new();
    let mut rest = prompt;
    while let Some(open) = rest.find(USER_FACT_OPEN) {
        let after = &rest[open + USER_FACT_OPEN.len()..];
        rest = after;
        if !after.starts_with(char::is_whitespace) {
            continue;
        }
        let Some(tag_end) = after.find('>') else {
            break;
        };
        let body_and_rest = &after[tag_end + 1..];
        let Some(close) = body_and_rest.find(USER_FACT_CLOSE) else {
            break;
        };
        let body = &body_and_rest[..close];
        if body.contains(USER_FACT_OPEN) {
            continue;
        }
        facts.push(body);
        rest = &body_and_rest[close + USER_FACT_CLOSE.len()..];
    }
    facts
}

/// Every maximal run of ASCII digits in `text`, in order.
fn digit_runs(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|run| !run.is_empty())
}

/// The last assistant message and its index in the assembled turn — the reply
/// the athlete is responding to.
fn previous_reply(llm_messages: &[ChatMessage]) -> Option<(usize, &ChatMessage)> {
    llm_messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| matches!(m.role, MessageRole::Assistant))
}

/// Whether the most recent assistant turn asserted concrete facts about the
/// athlete's own training.
///
/// Counts runs of digits rather than matching words, so it holds in every
/// locale and survives any rephrasing. Reads the last assistant message in the
/// assembled turn, which is the reply the athlete is responding to.
///
/// Pure and `pub` for the same reason `should_refresh_activity_context` is: the
/// decision is worth pinning without standing up an executor.
#[must_use]
pub fn previous_reply_asserted_athlete_facts(llm_messages: &[ChatMessage]) -> bool {
    previous_reply(llm_messages)
        .is_some_and(|(_, reply)| digit_runs(&reply.content).count() >= CLAIM_DENSITY_THRESHOLD)
}

/// [`previous_reply_asserted_athlete_facts`] for a turn an interview owns: a
/// figure the athlete supplied is not a claim.
///
/// An interview question acknowledges what it collected and quotes it back —
/// live 2026-09-30 on `/season`, «150 km (5 juin 2027) … (25–65 km)» restated
/// the athlete's own race calendar, and the plain count read it as a
/// reconstruction and re-asked (registre#678). Only a figure the athlete
/// never supplied is the agent's own assertion, so only those count: «161 km,
/// 2391 m, 6,2h le mois dernier» invented mid-walk still re-grounds when the
/// athlete disputes it.
///
/// The athlete supplied what is in their messages before the reply (not the
/// compaction summary, which a model wrote from assistant turns) and in
/// their dossier — the `<user_fact>` bodies of the system prompt, where a
/// walk's recorded answers and their goals live. The rest of the system
/// prompt (today's date, limits, instructions) is not the athlete's and
/// supplies nothing; earlier `assistant` turns supply nothing either, so a
/// figure the agent asserted once without grounding cannot vouch for itself
/// the second time.
#[must_use]
pub fn previous_reply_asserted_unsupplied_facts(llm_messages: &[ChatMessage]) -> bool {
    let Some((at, reply)) = previous_reply(llm_messages) else {
        return false;
    };
    let mut supplied = HashSet::new();
    for (index, message) in llm_messages.iter().enumerate() {
        match message.role {
            MessageRole::System => {
                for fact in dossier_facts(&message.content) {
                    supply(fact, &mut supplied);
                }
            }
            // The compaction summary rides in the User role but is written by
            // a model from earlier assistant turns: an ungrounded figure the
            // agent asserted must not come back through it as the athlete's.
            MessageRole::User
                if index < at && !message.content.starts_with(REPLAYED_SUMMARY_PREFIX) =>
            {
                supply(&message.content, &mut supplied);
            }
            _ => {}
        }
    }
    figures(&reply.content)
        .iter()
        .map(|figure| figure.unsupplied(&supplied))
        .sum::<usize>()
        >= CLAIM_DENSITY_THRESHOLD
}
