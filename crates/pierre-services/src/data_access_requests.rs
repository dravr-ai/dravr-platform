// ABOUTME: The athlete's statutory data access requests: write the letter, count the legal deadline, follow the state
// ABOUTME: Quebec P-39.1 (30 days, art. 32) or GDPR (one month, Art. 12(3)); Dravr writes the letter, the athlete sends it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Data access requests (carnet#1646, Owl row 60)
//!
//! A fallback for what Dravr cannot import: an athlete asks a company for the
//! personal information it holds about them, under Quebec's Act respecting
//! the protection of personal information in the private sector (CQLR
//! c. P-39.1) or the GDPR. Dravr writes the letter from the catalogue's
//! `dataAccess.letter.*` keys in the language the athlete picked; the athlete
//! copies it, downloads it or opens it in their mail client. **Dravr never
//! sends it**, and the company's address is the one the athlete entered.
//!
//! The lifecycle is `drafted` → `sent` → `answered`, or `closed` at any point
//! before an answer. A draft can be edited; a sent request can take a
//! corrected sending date and, under the GDPR, the later date a company
//! notified it would answer by (Art. 12(3), at most two further months).
//!
//! ## Counting the deadline
//!
//! [`due_on`] counts from the day the athlete says they sent it, which the
//! letter does not know, so the athlete may change it:
//!
//! - **Quebec**, art. 32: 30 calendar days.
//! - **GDPR**, Art. 12(3): one calendar month, the last day of a shorter month
//!   standing in for a day it does not have (31 January → 28 or 29 February);
//!   or the extension, when one was notified.
//!
//! Whether a sent request is *due soon* (three days or fewer) or *overdue* is
//! derived when it is read ([`derived_state`]), never stored.

use dravr_tronc::dates::{CivilDate, Timestamp};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_DATA_ACCESS_LETTER_ACCESS, KEY_DATA_ACCESS_LETTER_CLOSING,
    KEY_DATA_ACCESS_LETTER_DATE, KEY_DATA_ACCESS_LETTER_GDPR_COST,
    KEY_DATA_ACCESS_LETTER_GDPR_DEADLINE, KEY_DATA_ACCESS_LETTER_GDPR_OPENING,
    KEY_DATA_ACCESS_LETTER_GDPR_PORTABILITY, KEY_DATA_ACCESS_LETTER_GDPR_SALUTATION,
    KEY_DATA_ACCESS_LETTER_GDPR_SUBJECT, KEY_DATA_ACCESS_LETTER_IDENTIFIER,
    KEY_DATA_ACCESS_LETTER_QUEBEC_COST, KEY_DATA_ACCESS_LETTER_QUEBEC_DEADLINE,
    KEY_DATA_ACCESS_LETTER_QUEBEC_OPENING, KEY_DATA_ACCESS_LETTER_QUEBEC_PORTABILITY,
    KEY_DATA_ACCESS_LETTER_QUEBEC_SALUTATION, KEY_DATA_ACCESS_LETTER_QUEBEC_SUBJECT,
    KEY_DATA_ACCESS_LETTER_REFERENCE,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{TenantId, SUPPORTED_LOCALES};
use pierre_database::repositories::{
    DataAccessLaw, DataAccessRequest, DataAccessScope, DataAccessStatus, DUE_SOON_DAYS,
};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Calendar days a Quebec company has to answer (P-39.1 art. 32).
const QUEBEC_ANSWER_DAYS: i64 = 30;

/// Calendar months a GDPR controller has to answer (Art. 12(3)).
const GDPR_ANSWER_MONTHS: u32 = 1;

/// Further calendar months a GDPR controller may take when it notifies the
/// athlete within the first one (Art. 12(3)).
const GDPR_EXTENSION_MONTHS: u32 = 2;

/// How far back a sending date may be entered: a request older than a year
/// is long past every deadline this tracks.
const SENT_ON_MAX_AGE_DAYS: i64 = 365;

/// The longest company name a request keeps, in characters.
const MAX_CONTROLLER_CHARS: usize = 200;

/// The longest privacy address a request keeps, in characters: an email, a
/// web form's address or a postal address.
const MAX_CONTACT_CHARS: usize = 500;

/// The longest account identifier a request keeps, in characters.
const MAX_IDENTIFIER_CHARS: usize = 320;

/// The alphabet of a reference's random part: RFC 4648 base32.
const REFERENCE_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Characters in a reference's random part.
const REFERENCE_RANDOM_CHARS: usize = 6;

/// How many references are drawn before a create gives up on a collision.
const REFERENCE_ATTEMPTS: usize = 5;

/// The day `law` gives a company to answer a request sent on `sent_on`, or
/// the extension a GDPR controller notified when there is one.
///
/// The count starts on the day the athlete sent it. `None` only past the end
/// of the calendar, which no validated date reaches.
#[must_use]
pub fn due_on(
    law: DataAccessLaw,
    sent_on: CivilDate,
    extended_until: Option<CivilDate>,
) -> Option<CivilDate> {
    match law {
        DataAccessLaw::QuebecP39_1 => sent_on.checked_add_days(QUEBEC_ANSWER_DAYS),
        DataAccessLaw::Gdpr => {
            extended_until.or_else(|| sent_on.checked_add_months(GDPR_ANSWER_MONTHS))
        }
    }
}

/// The latest date a GDPR controller may extend a request sent on `sent_on`
/// to: one month and the two further ones.
#[must_use]
fn latest_extension(sent_on: CivilDate) -> Option<CivilDate> {
    sent_on.checked_add_months(GDPR_ANSWER_MONTHS + GDPR_EXTENSION_MONTHS)
}

/// What a sent request reads as on `today`, beyond its stored status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedState {
    /// Due within [`DUE_SOON_DAYS`] days, today included.
    DueSoon,
    /// Past its due date with no answer recorded.
    Overdue,
}

impl DerivedState {
    /// The wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DueSoon => "due_soon",
            Self::Overdue => "overdue",
        }
    }
}

/// Whether `request` is due soon or overdue on `today`; `None` for a request
/// that is not sent, or sent with more than [`DUE_SOON_DAYS`] days left.
#[must_use]
pub fn derived_state(request: &DataAccessRequest, today: CivilDate) -> Option<DerivedState> {
    if request.status != DataAccessStatus::Sent {
        return None;
    }
    let due = request.due_on?;
    let left = due.signed_duration_since(today).num_days();
    if left < 0 {
        Some(DerivedState::Overdue)
    } else if left <= DUE_SOON_DAYS {
        Some(DerivedState::DueSoon)
    } else {
        None
    }
}

/// What the athlete fills in to write a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestDetails {
    /// The company the request is addressed to.
    pub controller: String,
    /// Its privacy address.
    pub controller_contact: String,
    /// The account or email the athlete uses there.
    pub athlete_identifier: String,
    /// The law the request is made under.
    pub law: DataAccessLaw,
    /// What it asks for.
    pub scope: DataAccessScope,
    /// The language the letter is written in.
    pub letter_locale: String,
}

/// A change to a draft; every field left `None` keeps its value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftEdit {
    /// A new company name.
    pub controller: Option<String>,
    /// A new privacy address.
    pub controller_contact: Option<String>,
    /// A new account identifier.
    pub athlete_identifier: Option<String>,
    /// Another law.
    pub law: Option<DataAccessLaw>,
    /// Another scope.
    pub scope: Option<DataAccessScope>,
    /// Another letter language.
    pub letter_locale: Option<String>,
}

/// A trimmed text field, refused when empty or longer than `max` characters.
fn field(name: &str, value: &str, max: usize) -> AppResult<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::invalid_input(format!("{name} is required")));
    }
    if value.chars().count() > max {
        return Err(AppError::invalid_input(format!(
            "{name} is longer than {max} characters"
        )));
    }
    if value.chars().any(|c| c.is_control() && c != '\n') {
        return Err(AppError::invalid_input(format!(
            "{name} holds a control character"
        )));
    }
    Ok(value.to_owned())
}

/// A one-line field: [`field`], refusing a line break too.
fn line(name: &str, value: &str, max: usize) -> AppResult<String> {
    let value = field(name, value, max)?;
    if value.contains('\n') {
        return Err(AppError::invalid_input(format!("{name} must be one line")));
    }
    Ok(value)
}

/// A letter language the platform writes.
fn letter_locale(value: &str) -> AppResult<String> {
    SUPPORTED_LOCALES
        .into_iter()
        .find(|supported| *supported == value)
        .map(str::to_owned)
        .ok_or_else(|| AppError::invalid_input(format!("Unsupported letter language: {value}")))
}

/// `details`, validated and trimmed.
fn validated(details: &RequestDetails) -> AppResult<RequestDetails> {
    Ok(RequestDetails {
        controller: line("controller", &details.controller, MAX_CONTROLLER_CHARS)?,
        controller_contact: field(
            "controller_contact",
            &details.controller_contact,
            MAX_CONTACT_CHARS,
        )?,
        athlete_identifier: line(
            "athlete_identifier",
            &details.athlete_identifier,
            MAX_IDENTIFIER_CHARS,
        )?,
        law: details.law,
        scope: details.scope,
        letter_locale: letter_locale(&details.letter_locale)?,
    })
}

/// A fresh reference for a request written in `year`: `DRV-<year>-` then six
/// base32 characters.
#[must_use]
fn draw_reference(year: i32) -> String {
    use rand::Rng;

    let mut rng = rand::rng();
    let random: String = (0..REFERENCE_RANDOM_CHARS)
        .map(|_| char::from(REFERENCE_ALPHABET[rng.random_range(0..REFERENCE_ALPHABET.len())]))
        .collect();
    format!("DRV-{year}-{random}")
}

/// The refusal for an action the request's status does not allow.
fn wrong_status(action: &str, status: DataAccessStatus) -> AppError {
    AppError::new(
        ErrorCode::ResourceLocked,
        format!("A {} request cannot be {action}", status.as_str()),
    )
}

/// The refusal when the row changed between the read and the write.
fn changed_meanwhile() -> AppError {
    AppError::new(
        ErrorCode::ResourceLocked,
        "This request changed meanwhile; reload it and try again",
    )
}

/// The athlete's data access requests: every call is scoped to one athlete in
/// one tenant.
#[derive(Debug, Clone, Copy)]
pub struct DataAccessRequests<'repos> {
    /// The repositories the requests are kept in.
    repos: &'repos RepositoryRegistry,
    /// The session's tenant.
    tenant_id: TenantId,
    /// The athlete.
    user_id: Uuid,
}

impl<'repos> DataAccessRequests<'repos> {
    /// The requests of `user_id` in `tenant_id`.
    #[must_use]
    pub const fn new(
        repos: &'repos RepositoryRegistry,
        tenant_id: TenantId,
        user_id: Uuid,
    ) -> Self {
        Self {
            repos,
            tenant_id,
            user_id,
        }
    }

    /// The athlete's requests, newest first.
    ///
    /// # Errors
    /// Returns a database error when the read fails.
    pub async fn list(&self) -> AppResult<Vec<DataAccessRequest>> {
        self.repos
            .data_access_requests
            .list(self.tenant_id, self.user_id)
            .await
    }

    /// The athlete's request `id`.
    ///
    /// # Errors
    /// Returns not found when it is not theirs, or a database error.
    pub async fn get(&self, id: Uuid) -> AppResult<DataAccessRequest> {
        self.repos
            .data_access_requests
            .get(self.tenant_id, self.user_id, id)
            .await?
            .ok_or_else(|| AppError::not_found("Data access request"))
    }

    /// Write a draft from `details`, under a fresh reference.
    ///
    /// # Errors
    /// Returns invalid input for a missing, overlong or malformed field or an
    /// unsupported language, or a database error.
    pub async fn create(
        &self,
        details: &RequestDetails,
        now: Timestamp,
    ) -> AppResult<DataAccessRequest> {
        let details = validated(details)?;
        for _ in 0..REFERENCE_ATTEMPTS {
            let request = DataAccessRequest {
                id: Uuid::new_v4(),
                tenant_id: self.tenant_id,
                user_id: self.user_id,
                reference: draw_reference(now.utc_date().year()),
                controller: details.controller.clone(),
                controller_contact: details.controller_contact.clone(),
                athlete_identifier: details.athlete_identifier.clone(),
                law: details.law,
                scope: details.scope,
                letter_locale: details.letter_locale.clone(),
                status: DataAccessStatus::Drafted,
                sent_on: None,
                due_on: None,
                extended_until: None,
                answered_on: None,
                due_soon_notified_at: None,
                overdue_notified_at: None,
                created_at: now,
                updated_at: now,
            };
            if self.repos.data_access_requests.insert(&request).await? {
                return Ok(request);
            }
        }
        Err(AppError::internal(
            "Could not draw an unused data access request reference",
        ))
    }

    /// Apply `edit` to the draft `id`.
    ///
    /// # Errors
    /// Returns not found, a conflict once it is no longer a draft, invalid
    /// input as [`Self::create`], or a database error.
    pub async fn edit_draft(
        &self,
        id: Uuid,
        edit: &DraftEdit,
        now: Timestamp,
    ) -> AppResult<DataAccessRequest> {
        let request = self.get(id).await?;
        if request.status != DataAccessStatus::Drafted {
            return Err(wrong_status("edited", request.status));
        }
        let details = validated(&RequestDetails {
            controller: edit
                .controller
                .clone()
                .unwrap_or_else(|| request.controller.clone()),
            controller_contact: edit
                .controller_contact
                .clone()
                .unwrap_or_else(|| request.controller_contact.clone()),
            athlete_identifier: edit
                .athlete_identifier
                .clone()
                .unwrap_or_else(|| request.athlete_identifier.clone()),
            law: edit.law.unwrap_or(request.law),
            scope: edit.scope.unwrap_or(request.scope),
            letter_locale: edit
                .letter_locale
                .clone()
                .unwrap_or_else(|| request.letter_locale.clone()),
        })?;
        let edited = DataAccessRequest {
            controller: details.controller,
            controller_contact: details.controller_contact,
            athlete_identifier: details.athlete_identifier,
            law: details.law,
            scope: details.scope,
            letter_locale: details.letter_locale,
            updated_at: now,
            ..request
        };
        self.write(&edited, DataAccessStatus::Drafted, false).await
    }

    /// Record that the athlete sent the request on `sent_on`, or correct the
    /// sending date of one already sent; the due date follows.
    ///
    /// # Errors
    /// Returns not found, a conflict for an answered or closed request,
    /// invalid input for a date after `now`'s day or more than
    /// `SENT_ON_MAX_AGE_DAYS` before it, or a database error.
    pub async fn mark_sent(
        &self,
        id: Uuid,
        sent_on: CivilDate,
        now: Timestamp,
    ) -> AppResult<DataAccessRequest> {
        let request = self.get(id).await?;
        let expected = request.status;
        if !matches!(expected, DataAccessStatus::Drafted | DataAccessStatus::Sent) {
            return Err(wrong_status("marked sent", expected));
        }
        let today = now.utc_date();
        let oldest = today
            .checked_sub_days(SENT_ON_MAX_AGE_DAYS)
            .unwrap_or(CivilDate::MIN);
        if sent_on > today || sent_on < oldest {
            return Err(AppError::invalid_input(
                "The sending date must be today or within the past year",
            ));
        }
        // An extension notified against the old date no longer holds once the
        // date it was counted from moves.
        let extended_until = request
            .extended_until
            .filter(|_| request.sent_on == Some(sent_on));
        let due = due_on(request.law, sent_on, extended_until)
            .ok_or_else(|| AppError::invalid_input("The sending date is out of range"))?;
        let moved = request.due_on != Some(due);
        let sent = DataAccessRequest {
            status: DataAccessStatus::Sent,
            sent_on: Some(sent_on),
            due_on: Some(due),
            extended_until,
            updated_at: now,
            ..request
        };
        self.write(&sent, expected, moved).await
    }

    /// Record the later date a GDPR controller notified it would answer by.
    ///
    /// # Errors
    /// Returns not found, a conflict unless the request is sent, invalid
    /// input for a Quebec request (P-39.1 provides no extension) or a date
    /// not after the one-month due date or past the two further months, or a
    /// database error.
    pub async fn record_extension(
        &self,
        id: Uuid,
        extended_until: CivilDate,
        now: Timestamp,
    ) -> AppResult<DataAccessRequest> {
        let request = self.get(id).await?;
        if request.status != DataAccessStatus::Sent {
            return Err(wrong_status("extended", request.status));
        }
        if request.law != DataAccessLaw::Gdpr {
            return Err(AppError::invalid_input(
                "Only a GDPR request can be extended",
            ));
        }
        let sent_on = request
            .sent_on
            .ok_or_else(|| AppError::database("A sent data access request has no sending date"))?;
        let base = due_on(DataAccessLaw::Gdpr, sent_on, None);
        let latest = latest_extension(sent_on);
        if base.is_none_or(|base| extended_until <= base)
            || latest.is_none_or(|latest| extended_until > latest)
        {
            return Err(AppError::invalid_input(
                "An extension runs past the one-month due date, by at most two further months",
            ));
        }
        let moved = request.due_on != Some(extended_until);
        let extended = DataAccessRequest {
            extended_until: Some(extended_until),
            due_on: Some(extended_until),
            updated_at: now,
            ..request
        };
        self.write(&extended, DataAccessStatus::Sent, moved).await
    }

    /// Record that the company answered on `answered_on`.
    ///
    /// # Errors
    /// Returns not found, a conflict unless the request is sent, invalid
    /// input for a date before it was sent or after `now`'s day, or a
    /// database error.
    pub async fn mark_answered(
        &self,
        id: Uuid,
        answered_on: CivilDate,
        now: Timestamp,
    ) -> AppResult<DataAccessRequest> {
        let request = self.get(id).await?;
        if request.status != DataAccessStatus::Sent {
            return Err(wrong_status("marked answered", request.status));
        }
        if answered_on > now.utc_date() || request.sent_on.is_some_and(|sent| answered_on < sent) {
            return Err(AppError::invalid_input(
                "The answer date must fall between the sending date and today",
            ));
        }
        let answered = DataAccessRequest {
            status: DataAccessStatus::Answered,
            answered_on: Some(answered_on),
            updated_at: now,
            ..request
        };
        self.write(&answered, DataAccessStatus::Sent, false).await
    }

    /// Close a request the athlete gave up on, or that no longer matters.
    ///
    /// # Errors
    /// Returns not found, a conflict for one answered or closed already, or
    /// a database error.
    pub async fn close(&self, id: Uuid, now: Timestamp) -> AppResult<DataAccessRequest> {
        let request = self.get(id).await?;
        let expected = request.status;
        if !matches!(expected, DataAccessStatus::Drafted | DataAccessStatus::Sent) {
            return Err(wrong_status("closed", expected));
        }
        let closed = DataAccessRequest {
            status: DataAccessStatus::Closed,
            updated_at: now,
            ..request
        };
        self.write(&closed, expected, false).await
    }

    /// Write `request` while the stored row is still `expected`, and read it
    /// back.
    async fn write(
        &self,
        request: &DataAccessRequest,
        expected: DataAccessStatus,
        reset_reminders: bool,
    ) -> AppResult<DataAccessRequest> {
        if !self
            .repos
            .data_access_requests
            .update(request, expected, reset_reminders)
            .await?
        {
            return Err(changed_meanwhile());
        }
        self.get(request.id).await
    }
}

/// A rendered letter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Letter {
    /// The language it is written in.
    pub locale: String,
    /// The subject line: the `mailto:` subject and the text file's heading.
    pub subject: String,
    /// The letter itself, its paragraphs separated by a blank line.
    pub body: String,
}

impl Letter {
    /// The letter as one text file: the subject, a blank line, the body.
    #[must_use]
    pub fn text(&self) -> String {
        format!("{}\n\n{}\n", self.subject, self.body)
    }

    /// A `mailto:` link that opens the letter in the athlete's mail client,
    /// addressed to `contact` — or `None` when the address the athlete
    /// entered is not an email address (a web form, a postal address).
    #[must_use]
    pub fn mailto(&self, contact: &str) -> Option<String> {
        let contact = contact.trim();
        let (local, domain) = contact.split_once('@')?;
        let plain = |part: &str| {
            !part.is_empty()
                && part.chars().all(|c| {
                    !c.is_whitespace()
                        && !c.is_control()
                        && !matches!(c, '?' | '&' | '#' | '<' | '>' | ',')
                })
        };
        if !plain(local) || !plain(domain) || !domain.contains('.') || domain.contains('@') {
            return None;
        }
        Some(format!(
            "mailto:{contact}?subject={}&body={}",
            urlencoding::encode(&self.subject),
            urlencoding::encode(&self.body)
        ))
    }
}

/// Full month names, January first, in the five platform locales.
fn month_names(locale: &str) -> [&'static str; 12] {
    match locale {
        "en" => [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ],
        "es" => [
            "enero",
            "febrero",
            "marzo",
            "abril",
            "mayo",
            "junio",
            "julio",
            "agosto",
            "septiembre",
            "octubre",
            "noviembre",
            "diciembre",
        ],
        "de" => [
            "Januar",
            "Februar",
            "März",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ],
        "pt" => [
            "janeiro",
            "fevereiro",
            "março",
            "abril",
            "maio",
            "junho",
            "julho",
            "agosto",
            "setembro",
            "outubro",
            "novembro",
            "dezembro",
        ],
        _ => [
            "janvier",
            "février",
            "mars",
            "avril",
            "mai",
            "juin",
            "juillet",
            "août",
            "septembre",
            "octobre",
            "novembre",
            "décembre",
        ],
    }
}

/// `date` written out as a letter writes it in `locale`.
///
/// "18 novembre 2026", "November 18, 2026", "18 de noviembre de 2026",
/// "18. November 2026", "18 de novembro de 2026". French for a locale the
/// platform does not write, as the catalogue falls back.
#[must_use]
pub(crate) fn written_date(date: CivilDate, locale: &str) -> String {
    let month = month_names(locale)
        .get((date.month() - 1) as usize)
        .copied()
        .unwrap_or_default();
    let (day, year) = (date.day(), date.year());
    match locale {
        "en" => format!("{month} {day}, {year}"),
        "es" | "pt" => format!("{day} de {month} de {year}"),
        "de" => format!("{day}. {month} {year}"),
        _ => format!("{day} {month} {year}"),
    }
}

/// The per-law keys of a letter.
struct LawKeys {
    subject: &'static str,
    salutation: &'static str,
    opening: &'static str,
    portability: &'static str,
    deadline: &'static str,
    cost: &'static str,
}

const fn law_keys(law: DataAccessLaw) -> LawKeys {
    match law {
        DataAccessLaw::QuebecP39_1 => LawKeys {
            subject: KEY_DATA_ACCESS_LETTER_QUEBEC_SUBJECT,
            salutation: KEY_DATA_ACCESS_LETTER_QUEBEC_SALUTATION,
            opening: KEY_DATA_ACCESS_LETTER_QUEBEC_OPENING,
            portability: KEY_DATA_ACCESS_LETTER_QUEBEC_PORTABILITY,
            deadline: KEY_DATA_ACCESS_LETTER_QUEBEC_DEADLINE,
            cost: KEY_DATA_ACCESS_LETTER_QUEBEC_COST,
        },
        DataAccessLaw::Gdpr => LawKeys {
            subject: KEY_DATA_ACCESS_LETTER_GDPR_SUBJECT,
            salutation: KEY_DATA_ACCESS_LETTER_GDPR_SALUTATION,
            opening: KEY_DATA_ACCESS_LETTER_GDPR_OPENING,
            portability: KEY_DATA_ACCESS_LETTER_GDPR_PORTABILITY,
            deadline: KEY_DATA_ACCESS_LETTER_GDPR_DEADLINE,
            cost: KEY_DATA_ACCESS_LETTER_GDPR_COST,
        },
    }
}

/// Write the letter for `request` in `locale`, signed `athlete_name` and
/// dated the day it was sent, or `today` while it is a draft.
///
/// The paragraphs, in order: the date, the salutation to the company's
/// privacy officer, the law's opening, the access ask, the portability ask
/// (only for [`DataAccessScope::AccessPortability`]), the identifier line,
/// the deadline, the cost, the reference, and the closing with the name.
///
/// # Errors
/// Returns invalid input for a locale the platform does not write.
pub fn render_letter(
    strings: &MessagingStringsRegistry,
    request: &DataAccessRequest,
    locale: &str,
    athlete_name: &str,
    today: CivilDate,
) -> AppResult<Letter> {
    let locale = letter_locale(locale)?;
    let keys = law_keys(request.law);
    let say = |key: &str, args: &[&str]| strings.render(key, &locale, args);
    let date = written_date(request.sent_on.unwrap_or(today), &locale);
    let mut paragraphs = vec![
        say(KEY_DATA_ACCESS_LETTER_DATE, &[&date]),
        say(keys.salutation, &[&request.controller]),
        say(keys.opening, &[]),
        say(KEY_DATA_ACCESS_LETTER_ACCESS, &[]),
    ];
    if request.scope == DataAccessScope::AccessPortability {
        paragraphs.push(say(keys.portability, &[]));
    }
    paragraphs.extend([
        say(
            KEY_DATA_ACCESS_LETTER_IDENTIFIER,
            &[&request.athlete_identifier],
        ),
        say(keys.deadline, &[]),
        say(keys.cost, &[]),
        say(KEY_DATA_ACCESS_LETTER_REFERENCE, &[&request.reference]),
        say(KEY_DATA_ACCESS_LETTER_CLOSING, &[athlete_name]),
    ]);
    Ok(Letter {
        subject: say(keys.subject, &[&request.reference]),
        body: paragraphs.join("\n\n"),
        locale,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> CivilDate {
        CivilDate::from_ymd(y, m, d).unwrap()
    }

    #[test]
    fn quebec_counts_thirty_calendar_days() {
        assert_eq!(
            due_on(DataAccessLaw::QuebecP39_1, day(2026, 11, 18), None),
            Some(day(2026, 12, 18)),
            "bug: art. 32 is 30 days from the sending day"
        );
        assert_eq!(
            due_on(DataAccessLaw::QuebecP39_1, day(2026, 12, 15), None),
            Some(day(2027, 1, 14)),
            "bug: 30 days do not cross a year end"
        );
        assert_eq!(
            due_on(DataAccessLaw::QuebecP39_1, day(2028, 2, 1), None),
            Some(day(2028, 3, 2)),
            "bug: 30 days do not count a leap February's 29 days"
        );
        assert_eq!(
            due_on(
                DataAccessLaw::QuebecP39_1,
                day(2026, 11, 18),
                Some(day(2027, 1, 30))
            ),
            Some(day(2026, 12, 18)),
            "bug: a stray extension moved a Quebec due date"
        );
    }

    #[test]
    fn gdpr_counts_a_calendar_month_clamped_to_the_month_end() {
        assert_eq!(
            due_on(DataAccessLaw::Gdpr, day(2026, 11, 18), None),
            Some(day(2026, 12, 18)),
            "bug: one month after the 18th is the 18th"
        );
        assert_eq!(
            due_on(DataAccessLaw::Gdpr, day(2027, 1, 31), None),
            Some(day(2027, 2, 28)),
            "bug: 31 January runs to the last day of a common February"
        );
        assert_eq!(
            due_on(DataAccessLaw::Gdpr, day(2028, 1, 31), None),
            Some(day(2028, 2, 29)),
            "bug: 31 January runs to 29 February in a leap year"
        );
        assert_eq!(
            due_on(DataAccessLaw::Gdpr, day(2028, 1, 30), None),
            Some(day(2028, 2, 29)),
            "bug: 30 January runs to 29 February in a leap year"
        );
        assert_eq!(
            due_on(DataAccessLaw::Gdpr, day(2026, 3, 31), None),
            Some(day(2026, 4, 30)),
            "bug: 31 March runs to the last day of April"
        );
        assert_eq!(
            due_on(DataAccessLaw::Gdpr, day(2026, 12, 31), None),
            Some(day(2027, 1, 31)),
            "bug: a month from 31 December is 31 January"
        );
        assert_eq!(
            due_on(
                DataAccessLaw::Gdpr,
                day(2026, 11, 18),
                Some(day(2027, 2, 1))
            ),
            Some(day(2027, 2, 1)),
            "bug: a notified extension is the due date"
        );
    }

    #[test]
    fn a_gdpr_extension_reaches_three_months_at_most() {
        assert_eq!(
            latest_extension(day(2026, 11, 18)),
            Some(day(2027, 2, 18)),
            "bug: the latest extension is not three calendar months after sending"
        );
        assert_eq!(
            latest_extension(day(2026, 11, 30)),
            Some(day(2027, 2, 28)),
            "bug: three months from 30 November clamp to February's end"
        );
    }

    fn sent_due(due: CivilDate) -> DataAccessRequest {
        let at = Timestamp::now();
        DataAccessRequest {
            id: Uuid::new_v4(),
            tenant_id: TenantId::generate(),
            user_id: Uuid::new_v4(),
            reference: "DRV-2026-ABCDEF".to_owned(),
            controller: "Acme".to_owned(),
            controller_contact: "privacy@acme.example".to_owned(),
            athlete_identifier: "ana@example.com".to_owned(),
            law: DataAccessLaw::Gdpr,
            scope: DataAccessScope::Access,
            letter_locale: "en".to_owned(),
            status: DataAccessStatus::Sent,
            sent_on: Some(day(2026, 10, 18)),
            due_on: Some(due),
            extended_until: None,
            answered_on: None,
            due_soon_notified_at: None,
            overdue_notified_at: None,
            created_at: at,
            updated_at: at,
        }
    }

    #[test]
    fn due_soon_and_overdue_are_read_from_the_due_date() {
        let today = day(2026, 11, 18);
        assert_eq!(
            derived_state(&sent_due(day(2026, 11, 22)), today),
            None,
            "bug: four days left already reads due soon"
        );
        assert_eq!(
            derived_state(&sent_due(day(2026, 11, 21)), today),
            Some(DerivedState::DueSoon),
            "bug: three days left is due soon"
        );
        assert_eq!(
            derived_state(&sent_due(today), today),
            Some(DerivedState::DueSoon),
            "bug: the due day itself is still due soon, not overdue"
        );
        assert_eq!(
            derived_state(&sent_due(day(2026, 11, 17)), today),
            Some(DerivedState::Overdue),
            "bug: the day after the due date is overdue"
        );
        let mut answered = sent_due(day(2026, 11, 1));
        answered.status = DataAccessStatus::Answered;
        assert_eq!(
            derived_state(&answered, today),
            None,
            "bug: an answered request read as overdue"
        );
    }

    #[test]
    fn a_reference_is_drv_the_year_and_six_base32_characters() {
        let reference = draw_reference(2026);
        let random = reference
            .strip_prefix("DRV-2026-")
            .expect("bug: the reference does not open with DRV and the year");
        assert_eq!(
            random.len(),
            REFERENCE_RANDOM_CHARS,
            "bug: {reference} does not carry six random characters"
        );
        assert!(
            random.bytes().all(|b| REFERENCE_ALPHABET.contains(&b)),
            "bug: {reference} leaves the base32 alphabet"
        );
    }

    #[test]
    fn a_letter_date_is_written_in_its_language() {
        let date = day(2026, 11, 8);
        for (locale, written) in [
            ("fr", "8 novembre 2026"),
            ("en", "November 8, 2026"),
            ("es", "8 de noviembre de 2026"),
            ("de", "8. November 2026"),
            ("pt", "8 de novembro de 2026"),
        ] {
            assert_eq!(
                written_date(date, locale),
                written,
                "bug: a {locale} letter does not write the date as its language does"
            );
        }
    }

    #[test]
    fn mailto_is_offered_only_for_an_email_address() {
        let letter = Letter {
            locale: "en".to_owned(),
            subject: "Access request — reference DRV-2026-ABCDEF".to_owned(),
            body: "Line one\n\nLine & two".to_owned(),
        };
        let link = letter
            .mailto(" privacy@acme.example ")
            .expect("bug: an email address gets no mailto link");
        assert!(
            link.starts_with("mailto:privacy@acme.example?subject="),
            "bug: the link is not addressed to the trimmed contact: {link}"
        );
        assert!(
            link.contains("&body=Line%20one%0A%0ALine%20%26%20two"),
            "bug: the body is not percent-encoded: {link}"
        );
        assert_eq!(
            letter.mailto("https://acme.example/privacy"),
            None,
            "bug: a web form's address got a mailto link"
        );
        assert_eq!(
            letter.mailto("Acme, 1 Main St, Montréal"),
            None,
            "bug: a postal address got a mailto link"
        );
        assert_eq!(
            letter.mailto("privacy@acme.example?cc=x@y.z"),
            None,
            "bug: a contact carrying its own query got a mailto link"
        );
    }

    #[test]
    fn a_field_is_trimmed_and_bounded() {
        assert_eq!(
            line("controller", "  Acme  ", 10).unwrap(),
            "Acme",
            "bug: a field is not trimmed"
        );
        assert!(
            line("controller", "   ", 10).is_err(),
            "bug: a blank field was taken"
        );
        assert!(
            line("controller", &"x".repeat(11), 10).is_err(),
            "bug: a field over its ceiling was taken"
        );
        assert!(
            line("controller", "Acme\nInc", 10).is_err(),
            "bug: a one-line field took a line break"
        );
        assert_eq!(
            field("controller_contact", "1 Main St\nMontréal", 50).unwrap(),
            "1 Main St\nMontréal",
            "bug: a postal address may span lines"
        );
        assert!(
            letter_locale("it").is_err(),
            "bug: a language the platform does not write was taken"
        );
    }
}
