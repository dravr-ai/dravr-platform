// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The HealthKit read purpose string per app locale, read from the catalogue key providers.deviceHealth.permissionCopy
// ABOUTME: Throws when a locale's copy lacks the key, so a build never ships an English or empty sentence in its place

const fs = require('fs');
const path = require('path');

/** The pinned catalogue copies, written only by the contremaitre bump lane: one directory per app locale. */
const LOCALES_DIR = path.join(__dirname, '..', '..', 'packages', 'i18n', 'src', 'locales');

/** The locale whose sentence is the base Info.plist string; every other becomes `<lang>.lproj/InfoPlist.strings`. */
const BASE_LOCALE = 'en';

/**
 * The sentence Apple's Health sheet shows in `locale`. InfoPlist.strings is
 * written verbatim between double quotes, so a quote, backslash or line break
 * would corrupt the file: those are refused rather than escaped.
 */
function healthSharePurpose(locale) {
  const catalogue = require(path.join(LOCALES_DIR, locale, 'translation.json'));
  const copy = catalogue?.providers?.deviceHealth?.permissionCopy;
  if (typeof copy !== 'string' || copy.trim() === '') {
    throw new Error(`packages/i18n ${locale}/translation.json has no providers.deviceHealth.permissionCopy`);
  }
  if (/["\\\n\r]/.test(copy)) {
    throw new Error(`providers.deviceHealth.permissionCopy (${locale}) holds a quote, backslash or line break InfoPlist.strings cannot carry`);
  }
  return copy;
}

/** Expo `locales` entries for every non-base locale, iOS-only so Android gets no strings.xml for them. */
function healthShareLocales() {
  return Object.fromEntries(
    fs
      .readdirSync(LOCALES_DIR, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && entry.name !== BASE_LOCALE)
      .map(({ name: locale }) => [locale, { ios: { NSHealthShareUsageDescription: healthSharePurpose(locale) } }]),
  );
}

module.exports = { BASE_LOCALE, healthSharePurpose, healthShareLocales };
