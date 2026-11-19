// ABOUTME: The locale what a channel tells us about a sender suggests: a phone number, a WhatsApp BSUID, a language tag
// ABOUTME: One country-to-locale table behind all three; English for a country or a language the platform does not speak
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What a sender with no account tells us about their language.
//!
//! A sender who has not linked an account yet is known only by what their
//! channel hands over: a `WhatsApp` phone number or business-scoped user id
//! (BSUID), a Telegram app's language tag. Each is a better guess at the
//! language they read than the platform default, so the linking prompts they
//! get before any account exists are chosen from it (carnet#1669).
//!
//! A phone number and a BSUID both name a country, and one table maps a
//! country to its locale, so the two cannot disagree. The +1 numbering plan
//! spans Canada, the United States and the Caribbean, so a number in it is
//! split by area code: Quebec's read French, Puerto Rico's and the Dominican
//! Republic's read Spanish, and every other one reads English. A BSUID names
//! only the country, so a Canadian BSUID reads English, the language most of
//! Canada reads.
//!
//! A sender placed in a country, or by a language, the platform does not
//! speak reads English (owner decision, carnet#1669). Only a sender nothing
//! places is left to the caller's default.

use crate::models::supported_ui_locale;

/// English: the locale of every country listed nowhere below, which is the
/// English-speaking ones and, by owner decision (carnet#1669), any whose
/// language the platform does not speak.
const ENGLISH_LOCALE: &str = "en";

/// What a +1 number from a Quebec area code reads.
const QUEBEC_LOCALE: &str = "fr";

/// Countries (ISO 3166 alpha-2) whose language the platform speaks and is not
/// English, with that locale. Every other country reads [`ENGLISH_LOCALE`].
const COUNTRY_LOCALES: &[(&str, &str)] = &[
    // French
    ("FR", "fr"), // France
    ("BE", "fr"), // Belgium
    ("LU", "fr"), // Luxembourg
    ("MC", "fr"), // Monaco
    ("RE", "fr"), // Réunion
    ("YT", "fr"), // Mayotte
    ("PM", "fr"), // Saint Pierre and Miquelon
    ("HT", "fr"), // Haiti
    ("GP", "fr"), // Guadeloupe
    ("BL", "fr"), // Saint Barthélemy
    ("MF", "fr"), // Saint Martin
    ("GF", "fr"), // French Guiana
    ("MQ", "fr"), // Martinique
    ("WF", "fr"), // Wallis and Futuna
    ("NC", "fr"), // New Caledonia
    ("PF", "fr"), // French Polynesia
    ("SN", "fr"), // Senegal
    ("ML", "fr"), // Mali
    ("GN", "fr"), // Guinea
    ("CI", "fr"), // Côte d'Ivoire
    ("BF", "fr"), // Burkina Faso
    ("NE", "fr"), // Niger
    ("TG", "fr"), // Togo
    ("BJ", "fr"), // Benin
    ("TD", "fr"), // Chad
    ("CF", "fr"), // Central African Republic
    ("CM", "fr"), // Cameroon
    ("GA", "fr"), // Gabon
    ("CG", "fr"), // Republic of the Congo
    ("CD", "fr"), // Democratic Republic of the Congo
    ("DJ", "fr"), // Djibouti
    ("BI", "fr"), // Burundi
    ("MG", "fr"), // Madagascar
    ("KM", "fr"), // Comoros
    // German
    ("DE", "de"), // Germany
    ("AT", "de"), // Austria
    ("CH", "de"), // Switzerland, which reads German more than any other language
    ("LI", "de"), // Liechtenstein
    // Spanish
    ("ES", "es"), // Spain
    ("MX", "es"), // Mexico
    ("CU", "es"), // Cuba
    ("PE", "es"), // Peru
    ("AR", "es"), // Argentina
    ("CL", "es"), // Chile
    ("CO", "es"), // Colombia
    ("VE", "es"), // Venezuela
    ("GQ", "es"), // Equatorial Guinea
    ("GT", "es"), // Guatemala
    ("SV", "es"), // El Salvador
    ("HN", "es"), // Honduras
    ("NI", "es"), // Nicaragua
    ("CR", "es"), // Costa Rica
    ("PA", "es"), // Panama
    ("BO", "es"), // Bolivia
    ("EC", "es"), // Ecuador
    ("PY", "es"), // Paraguay
    ("UY", "es"), // Uruguay
    ("PR", "es"), // Puerto Rico
    ("DO", "es"), // Dominican Republic
    // Portuguese
    ("PT", "pt"), // Portugal
    ("BR", "pt"), // Brazil
    ("CV", "pt"), // Cape Verde
    ("ST", "pt"), // São Tomé and Príncipe
    ("AO", "pt"), // Angola
    ("GW", "pt"), // Guinea-Bissau
    ("MZ", "pt"), // Mozambique
    ("TL", "pt"), // Timor-Leste
];

/// Country calling codes outside the +1 plan, for the countries
/// [`COUNTRY_LOCALES`] lists; a code shared by several territories names the
/// one it was assigned to first.
///
/// ITU calling codes are prefix-free, so a number starts with at most one of
/// them and the order here does not matter.
const CALLING_CODE_COUNTRIES: &[(&str, &str)] = &[
    ("33", "FR"),
    ("32", "BE"),
    ("41", "CH"),
    ("352", "LU"),
    ("377", "MC"),
    ("262", "RE"),
    ("508", "PM"),
    ("509", "HT"),
    ("590", "GP"),
    ("594", "GF"),
    ("596", "MQ"),
    ("681", "WF"),
    ("687", "NC"),
    ("689", "PF"),
    ("221", "SN"),
    ("223", "ML"),
    ("224", "GN"),
    ("225", "CI"),
    ("226", "BF"),
    ("227", "NE"),
    ("228", "TG"),
    ("229", "BJ"),
    ("235", "TD"),
    ("236", "CF"),
    ("237", "CM"),
    ("241", "GA"),
    ("242", "CG"),
    ("243", "CD"),
    ("253", "DJ"),
    ("257", "BI"),
    ("261", "MG"),
    ("269", "KM"),
    ("49", "DE"),
    ("43", "AT"),
    ("423", "LI"),
    ("34", "ES"),
    ("51", "PE"),
    ("52", "MX"),
    ("53", "CU"),
    ("54", "AR"),
    ("56", "CL"),
    ("57", "CO"),
    ("58", "VE"),
    ("240", "GQ"),
    ("502", "GT"),
    ("503", "SV"),
    ("504", "HN"),
    ("505", "NI"),
    ("506", "CR"),
    ("507", "PA"),
    ("591", "BO"),
    ("593", "EC"),
    ("595", "PY"),
    ("598", "UY"),
    ("351", "PT"),
    ("55", "BR"),
    ("238", "CV"),
    ("239", "ST"),
    ("244", "AO"),
    ("245", "GW"),
    ("258", "MZ"),
    ("670", "TL"),
];

/// Area codes of the +1 numbering plan that ring in Quebec.
const QUEBEC_AREA_CODES: [&str; 12] = [
    "263", "354", "367", "418", "438", "450", "468", "514", "579", "581", "819", "873",
];

/// Area codes of the +1 numbering plan that ring in a country
/// [`COUNTRY_LOCALES`] lists: Puerto Rico and the Dominican Republic.
const PLAN_AREA_CODE_COUNTRIES: &[(&str, &str)] = &[
    ("787", "PR"),
    ("939", "PR"),
    ("809", "DO"),
    ("829", "DO"),
    ("849", "DO"),
];

/// The digits an international phone number is written with, the longest
/// E.164 allows.
const MAX_E164_DIGITS: usize = 15;

/// The fewest digits a full international number carries (Niue's are seven).
const MIN_E164_DIGITS: usize = 7;

/// Longest identifier Meta puts after a BSUID's country prefix.
const MAX_BSUID_IDENTIFIER: usize = 128;

/// Infix a parent BSUID carries between its country code and identifier.
const BSUID_PARENT_INFIX: &str = "ENT.";

/// The locale a country (ISO 3166 alpha-2) reads.
fn country_locale(country: &str) -> &'static str {
    COUNTRY_LOCALES
        .iter()
        .find(|(listed, _)| *listed == country)
        .map_or(ENGLISH_LOCALE, |(_, locale)| *locale)
}

/// The locale a phone number's country suggests, or `None` when `number` is
/// not an international phone number.
///
/// Takes the number in international form, digits only with an optional
/// leading `+`, the shape `WhatsApp` gives a sender's number. Anything else (a
/// username, a BSUID, a number too short or too long to be E.164, or one
/// starting with 0, which no calling code does) answers `None`. `WhatsApp`
/// hands over only numbers it delivers to, so a well-formed one always names a
/// country: one whose language the platform does not speak reads English.
#[must_use]
fn locale_from_phone_number(number: &str) -> Option<&'static str> {
    let digits = number.strip_prefix('+').unwrap_or(number);
    if !(MIN_E164_DIGITS..=MAX_E164_DIGITS).contains(&digits.len())
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || digits.starts_with('0')
    {
        return None;
    }
    if let Some(national) = digits.strip_prefix('1') {
        let area_code = national.get(..3).unwrap_or_default();
        if QUEBEC_AREA_CODES.contains(&area_code) {
            return Some(QUEBEC_LOCALE);
        }
        let locale = PLAN_AREA_CODE_COUNTRIES
            .iter()
            .find(|(area, _)| *area == area_code)
            .map_or(ENGLISH_LOCALE, |(_, country)| country_locale(country));
        return Some(locale);
    }
    let locale = CALLING_CODE_COUNTRIES
        .iter()
        .find(|(code, _)| digits.starts_with(code))
        .map_or(ENGLISH_LOCALE, |(_, country)| country_locale(country));
    Some(locale)
}

/// The locale the country of a `WhatsApp` business-scoped user id suggests,
/// or `None` when `id` is not one.
///
/// A BSUID is an ISO 3166 alpha-2 country code, a period, and up to 128
/// alphanumeric characters (`US.13491208655302741918`); a parent BSUID puts
/// `ENT.` after the period (`US.ENT.11815799212886844830`). It names a country
/// and no region, so a Canadian one reads English.
#[must_use]
fn locale_from_business_scoped_user_id(id: &str) -> Option<&'static str> {
    let (country, rest) = id.split_once('.')?;
    let identifier = rest.strip_prefix(BSUID_PARENT_INFIX).unwrap_or(rest);
    let is_bsuid = country.len() == 2
        && country.bytes().all(|byte| byte.is_ascii_uppercase())
        && !identifier.is_empty()
        && identifier.len() <= MAX_BSUID_IDENTIFIER
        && identifier.bytes().all(|byte| byte.is_ascii_alphanumeric());
    is_bsuid.then(|| country_locale(country))
}

/// The locale a `WhatsApp` sender's identifiers suggest, the most precise
/// first, or `None` when none of them places the sender.
///
/// 1. `phone_number`, the number Meta sent in `from` when the sender id is
///    something else: down to the area code.
/// 2. `sender_id` when it is itself a phone number, as every sender id was
///    before Meta's business-scoped user ids.
/// 3. The country of `sender_id` when it is a business-scoped user id.
#[must_use]
pub fn whatsapp_sender_locale(sender_id: &str, phone_number: Option<&str>) -> Option<&'static str> {
    phone_number
        .and_then(locale_from_phone_number)
        .or_else(|| locale_from_phone_number(sender_id))
        .or_else(|| locale_from_business_scoped_user_id(sender_id))
}

/// The locale a language tag (`en`, `pt-br`) names, or `None` for an empty
/// one.
///
/// A tag whose language the platform does not speak still says something
/// about the sender, and reads English.
#[must_use]
pub fn locale_from_language_tag(tag: &str) -> Option<&'static str> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    Some(supported_ui_locale(Some(tag)).unwrap_or(ENGLISH_LOCALE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SUPPORTED_LOCALES;

    #[test]
    fn every_quebec_area_code_reads_french() {
        for area_code in QUEBEC_AREA_CODES {
            let number = format!("1{area_code}5550123");
            assert_eq!(
                locale_from_phone_number(&number),
                Some("fr"),
                "bug: a Quebec number ({area_code}) does not read French"
            );
        }
    }

    #[test]
    fn the_rest_of_north_america_reads_english_except_its_spanish_islands() {
        for (number, expected) in [
            ("16135550123", "en"),  // Ottawa, across the river from Gatineau
            ("14165550123", "en"),  // Toronto
            ("16045550123", "en"),  // Vancouver
            ("15065550123", "en"),  // New Brunswick
            ("12125550123", "en"),  // New York
            ("18765550123", "en"),  // Jamaica
            ("17875550123", "es"),  // Puerto Rico
            ("18095550123", "es"),  // Dominican Republic
            ("+14385550123", "fr"), // Montreal, written with its plus
        ] {
            assert_eq!(
                locale_from_phone_number(number),
                Some(expected),
                "bug: {number} does not read {expected}"
            );
        }
    }

    #[test]
    fn a_country_reads_its_own_language() {
        for (number, expected) in [
            ("33612345678", "fr"),   // France
            ("32470123456", "fr"),   // Belgium
            ("41791234567", "de"),   // Switzerland
            ("2250701234567", "fr"), // Côte d'Ivoire
            ("34612345678", "es"),   // Spain
            ("5215512345678", "es"), // Mexico
            ("4915112345678", "de"), // Germany
            ("436641234567", "de"),  // Austria
            ("351912345678", "pt"),  // Portugal
            ("5511912345678", "pt"), // Brazil
            ("447700900123", "en"),  // United Kingdom
            ("61412345678", "en"),   // Australia
            ("353851234567", "en"),  // Ireland
        ] {
            assert_eq!(
                locale_from_phone_number(number),
                Some(expected),
                "bug: {number} does not read {expected}"
            );
        }
    }

    /// Owner decision (carnet#1669): a sender we can place in a country whose
    /// language the platform does not speak reads English, not the default.
    #[test]
    fn a_country_whose_language_the_platform_does_not_speak_reads_english() {
        for number in [
            "31612345678",  // Netherlands
            "393123456789", // Italy
            "46701234567",  // Sweden
            "81312345678",  // Japan
            "79161234567",  // Russia
        ] {
            assert_eq!(
                locale_from_phone_number(number),
                Some("en"),
                "bug: {number} was not answered in English"
            );
        }
    }

    #[test]
    fn what_is_not_an_international_number_is_no_signal() {
        for sender in [
            "",
            "unknown",
            "wa-15551234567",
            "+",
            "1514",             // too short to be a whole number
            "1514555012345678", // longer than E.164 allows
            "1514 555 0123",
            "0612345678", // a national number: no calling code starts with 0
            "US.13491208655302741918",
        ] {
            assert_eq!(
                locale_from_phone_number(sender),
                None,
                "bug: {sender:?} was read as a phone number"
            );
        }
    }

    #[test]
    fn a_business_scoped_user_id_reads_the_language_of_its_country() {
        for (bsuid, expected) in [
            ("FR.13491208655302741918", "fr"),
            ("BE.1349120865530274", "fr"),
            ("CH.9", "de"),
            ("DE.abc123", "de"),
            ("MX.ENT.11815799212886844830", "es"),
            ("PR.42", "es"),
            ("BR.42", "pt"),
            ("US.13491208655302741918", "en"),
            ("CA.13491208655302741918", "en"), // no area code to find Quebec by
            ("NL.42", "en"),
            ("JP.42", "en"),
        ] {
            assert_eq!(
                locale_from_business_scoped_user_id(bsuid),
                Some(expected),
                "bug: {bsuid} does not read {expected}"
            );
        }
    }

    #[test]
    fn what_is_not_a_business_scoped_user_id_is_no_signal() {
        let too_long = format!("US.{}", "1".repeat(MAX_BSUID_IDENTIFIER + 1));
        for id in [
            "",
            "15145550123",
            "us.13491208655302741918",
            "USA.13491208655302741918",
            "U.13491208655302741918",
            "US.",
            "US.ENT.",
            "US.1349-1208",
            too_long.as_str(),
        ] {
            assert_eq!(
                locale_from_business_scoped_user_id(id),
                None,
                "bug: {id:?} was read as a business-scoped user id"
            );
        }
    }

    #[test]
    fn a_whatsapp_sender_is_placed_by_their_most_precise_identifier() {
        for (sender_id, phone_number, expected) in [
            // A BSUID names the United States, the number Montreal.
            ("US.13491208655302741918", Some("15145550123"), Some("fr")),
            // A BSUID naming Canada and a number in Toronto.
            ("CA.13491208655302741918", Some("14165550123"), Some("en")),
            // Before BSUIDs the sender id was the number itself.
            ("15145550123", None, Some("fr")),
            // A BSUID with no number names only its country.
            ("CH.13491208655302741918", None, Some("de")),
            // A number Meta sent malformed leaves the BSUID's country.
            ("FR.13491208655302741918", Some("not-a-number"), Some("fr")),
            ("unknown", None, None),
        ] {
            assert_eq!(
                whatsapp_sender_locale(sender_id, phone_number),
                expected,
                "bug: sender {sender_id} with number {phone_number:?} does not read {expected:?}"
            );
        }
    }

    #[test]
    fn a_language_tag_reads_its_language_and_english_when_unspoken() {
        for (tag, expected) in [
            ("en", Some("en")),
            ("pt-br", Some("pt")),
            ("DE", Some("de")),
            ("nl", Some("en")),
            ("", None),
            ("  ", None),
        ] {
            assert_eq!(
                locale_from_language_tag(tag),
                expected,
                "bug: language tag {tag:?} does not read {expected:?}"
            );
        }
    }

    /// The lookup takes the first code a number starts with, which is the
    /// only one only while no listed code is a prefix of another.
    #[test]
    fn no_calling_code_is_a_prefix_of_another() {
        for (code, _) in CALLING_CODE_COUNTRIES {
            for (other, _) in CALLING_CODE_COUNTRIES {
                assert!(
                    code == other || !other.starts_with(code),
                    "bug: calling code {code} is a prefix of {other}, so {other} is unreachable"
                );
            }
            assert!(
                !code.starts_with('1'),
                "bug: calling code {code} sits inside the +1 plan the area codes split"
            );
        }
    }

    /// A number and a BSUID from one country read the same language because
    /// both go through the country table: every country a calling code or an
    /// area code names must be in it.
    #[test]
    fn every_country_a_number_names_is_in_the_country_table() {
        let named = CALLING_CODE_COUNTRIES
            .iter()
            .chain(PLAN_AREA_CODE_COUNTRIES)
            .map(|(_, country)| *country);
        for country in named {
            assert!(
                COUNTRY_LOCALES.iter().any(|(listed, _)| *listed == country),
                "bug: a number names {country}, which the country table does not list"
            );
        }
    }

    #[test]
    fn every_locale_the_tables_name_is_one_the_platform_speaks() {
        let named = COUNTRY_LOCALES
            .iter()
            .map(|(_, locale)| *locale)
            .chain([ENGLISH_LOCALE, QUEBEC_LOCALE]);
        for locale in named {
            assert!(
                SUPPORTED_LOCALES.contains(&locale),
                "bug: {locale} is not a supported locale"
            );
        }
        for (country, _) in COUNTRY_LOCALES {
            assert!(
                country.len() == 2 && country.bytes().all(|byte| byte.is_ascii_uppercase()),
                "bug: {country} is not an ISO 3166 alpha-2 code"
            );
        }
    }
}
