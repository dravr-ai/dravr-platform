// ABOUTME: The locale a phone number's country suggests, for a sender who has told us nothing else
// ABOUTME: Country calling code to a supported locale; North American +1 split by area code into fr, es and en
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A phone number as a language signal.
//!
//! A `WhatsApp` sender who has not linked an account yet is known by one thing:
//! their number. Its country calling code is a better guess at the language
//! they read than the platform default, so the linking prompts they get before
//! any account exists are chosen from it (carnet#1669).
//!
//! Only countries whose language the platform speaks are listed: a number from
//! anywhere else names no locale, and the caller keeps its default. The +1
//! numbering plan spans Canada, the United States and the Caribbean, so it is
//! split by area code: Quebec's read French, Puerto Rico's and the Dominican
//! Republic's read Spanish, and every other one reads English.

/// Area codes of the +1 numbering plan that ring in Quebec.
const QUEBEC_AREA_CODES: [&str; 12] = [
    "263", "354", "367", "418", "438", "450", "468", "514", "579", "581", "819", "873",
];

/// Area codes of the +1 numbering plan that ring in Puerto Rico (787, 939)
/// and the Dominican Republic (809, 829, 849).
const SPANISH_AREA_CODES: [&str; 5] = ["787", "939", "809", "829", "849"];

/// Country calling codes outside the +1 plan whose country reads a language
/// the platform speaks, with that locale.
///
/// ITU calling codes are prefix-free, so a number starts with at most one of
/// them and the order here does not matter.
const CALLING_CODE_LOCALES: &[(&str, &str)] = &[
    // French
    ("33", "fr"),  // France
    ("32", "fr"),  // Belgium
    ("41", "fr"),  // Switzerland
    ("352", "fr"), // Luxembourg
    ("377", "fr"), // Monaco
    ("262", "fr"), // Réunion, Mayotte
    ("508", "fr"), // Saint Pierre and Miquelon
    ("509", "fr"), // Haiti
    ("590", "fr"), // Guadeloupe
    ("594", "fr"), // French Guiana
    ("596", "fr"), // Martinique
    ("681", "fr"), // Wallis and Futuna
    ("687", "fr"), // New Caledonia
    ("689", "fr"), // French Polynesia
    ("221", "fr"), // Senegal
    ("223", "fr"), // Mali
    ("224", "fr"), // Guinea
    ("225", "fr"), // Côte d'Ivoire
    ("226", "fr"), // Burkina Faso
    ("227", "fr"), // Niger
    ("228", "fr"), // Togo
    ("229", "fr"), // Benin
    ("235", "fr"), // Chad
    ("236", "fr"), // Central African Republic
    ("237", "fr"), // Cameroon
    ("241", "fr"), // Gabon
    ("242", "fr"), // Republic of the Congo
    ("243", "fr"), // Democratic Republic of the Congo
    ("253", "fr"), // Djibouti
    ("257", "fr"), // Burundi
    ("261", "fr"), // Madagascar
    ("269", "fr"), // Comoros
    // Spanish
    ("34", "es"),  // Spain
    ("51", "es"),  // Peru
    ("52", "es"),  // Mexico
    ("53", "es"),  // Cuba
    ("54", "es"),  // Argentina
    ("56", "es"),  // Chile
    ("57", "es"),  // Colombia
    ("58", "es"),  // Venezuela
    ("240", "es"), // Equatorial Guinea
    ("502", "es"), // Guatemala
    ("503", "es"), // El Salvador
    ("504", "es"), // Honduras
    ("505", "es"), // Nicaragua
    ("506", "es"), // Costa Rica
    ("507", "es"), // Panama
    ("591", "es"), // Bolivia
    ("593", "es"), // Ecuador
    ("595", "es"), // Paraguay
    ("598", "es"), // Uruguay
    // German
    ("49", "de"),  // Germany
    ("43", "de"),  // Austria
    ("423", "de"), // Liechtenstein
    // Portuguese
    ("351", "pt"), // Portugal
    ("55", "pt"),  // Brazil
    ("238", "pt"), // Cape Verde
    ("239", "pt"), // São Tomé and Príncipe
    ("244", "pt"), // Angola
    ("245", "pt"), // Guinea-Bissau
    ("258", "pt"), // Mozambique
    ("670", "pt"), // Timor-Leste
    // English
    ("44", "en"),  // United Kingdom
    ("353", "en"), // Ireland
    ("356", "en"), // Malta
    ("61", "en"),  // Australia
    ("64", "en"),  // New Zealand
    ("27", "en"),  // South Africa
    ("63", "en"),  // Philippines
    ("65", "en"),  // Singapore
    ("91", "en"),  // India
    ("233", "en"), // Ghana
    ("234", "en"), // Nigeria
    ("254", "en"), // Kenya
    ("256", "en"), // Uganda
    ("260", "en"), // Zambia
];

/// The digits an international phone number is written with, the longest
/// E.164 allows.
const MAX_E164_DIGITS: usize = 15;

/// The fewest digits a full international number carries (Niue's are seven).
const MIN_E164_DIGITS: usize = 7;

/// The locale a phone number's country suggests, or `None` when the number
/// names no country whose language the platform speaks.
///
/// Takes the number in international form, digits only with an optional
/// leading `+` — the shape a `WhatsApp` sender id arrives in. Anything else (a
/// username, a business-scoped id, a number too short or too long to be
/// E.164) is no signal and answers `None`.
#[must_use]
pub fn locale_from_phone_number(number: &str) -> Option<&'static str> {
    let digits = number.strip_prefix('+').unwrap_or(number);
    if !(MIN_E164_DIGITS..=MAX_E164_DIGITS).contains(&digits.len())
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    if let Some(national) = digits.strip_prefix('1') {
        let area_code = national.get(..3).unwrap_or_default();
        let locale = if QUEBEC_AREA_CODES.contains(&area_code) {
            "fr"
        } else if SPANISH_AREA_CODES.contains(&area_code) {
            "es"
        } else {
            "en"
        };
        return Some(locale);
    }
    CALLING_CODE_LOCALES
        .iter()
        .find(|(code, _)| digits.starts_with(code))
        .map(|(_, locale)| *locale)
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
            ("41791234567", "fr"),   // Switzerland
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

    #[test]
    fn a_country_whose_language_the_platform_does_not_speak_names_no_locale() {
        for number in [
            "31612345678",  // Netherlands
            "393123456789", // Italy
            "81312345678",  // Japan
            "79161234567",  // Russia
        ] {
            assert_eq!(
                locale_from_phone_number(number),
                None,
                "bug: {number} named a locale its country does not read"
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
        ] {
            assert_eq!(
                locale_from_phone_number(sender),
                None,
                "bug: {sender:?} was read as a phone number"
            );
        }
    }

    /// The lookup takes the first code a number starts with, which is the
    /// only one only while no listed code is a prefix of another.
    #[test]
    fn no_calling_code_is_a_prefix_of_another() {
        for (code, _) in CALLING_CODE_LOCALES {
            for (other, _) in CALLING_CODE_LOCALES {
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

    #[test]
    fn every_locale_a_country_names_is_one_the_platform_speaks() {
        for (code, locale) in CALLING_CODE_LOCALES {
            assert!(
                SUPPORTED_LOCALES.contains(locale),
                "bug: calling code {code} names {locale}, which is not a supported locale"
            );
        }
    }
}
