// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Guards the client string corpus the way CI guards the server's — every key, every locale
// ABOUTME: A locale offered in the switcher but missing keys would ship an advertised-but-empty language

import { describe, it, expect } from 'vitest';
import { SUPPORTED_LANGUAGES, defaultI18nConfig, DEFAULT_LANGUAGE } from '@pierre/i18n';

/** Flatten a translation bundle to its dot-notation leaf keys. */
function leafKeys(bundle: unknown, prefix = ''): string[] {
  if (typeof bundle !== 'object' || bundle === null) {
    return [prefix];
  }
  return Object.entries(bundle as Record<string, unknown>).flatMap(([key, value]) =>
    leafKeys(value, prefix === '' ? key : `${prefix}.${key}`),
  );
}

function bundleFor(language: string): Record<string, unknown> {
  const resources = defaultI18nConfig.resources as Record<string, { translation: unknown }>;
  return resources[language].translation as Record<string, unknown>;
}

// Cognates and brand names — genuinely identical across languages, not
// untranslated leftovers. Anything else matching English is a gap.
const SHARED_ACROSS_LANGUAGES = new Set([
  'common.appName',
  'settings.claudeDesktop',
  'providers.strava',
  'providers.garmin',
  'providers.fitbit',
  'providers.polar',
]);

describe('client locale corpus', () => {
  it('offers exactly the five locales the server accepts, French first', () => {
    expect([...SUPPORTED_LANGUAGES]).toEqual(['fr', 'en', 'es', 'de', 'pt']);
    expect(DEFAULT_LANGUAGE).toBe('fr');
    expect(defaultI18nConfig.lng).toBe('fr');
    expect(defaultI18nConfig.fallbackLng).toBe('fr');
  });

  it('carries the same key set in every locale', () => {
    // 201 before the Chat-First Cutover retired the 23-key `social`
    // namespace with the feature it named, and 178 until the 14-key
    // `insights` namespace was dropped for the same reason — it outlived the
    // surface it named. The 7-key `nav` namespace is what the sidebar reads,
    // and Settings brought the rest across: profile, password, tokens,
    // credentials, account and about, plus the provider strings that
    // surface owns. The number only moves when a surface does.
    //
    // 1061 until `notifPrefs.loadFailedMobile` joined it: the notification
    // preferences screen exists on both surfaces and shares every other string,
    // but its load failure cannot — the web copy says "reload the page", and a
    // phone has no page to reload. One key, five locales, because a surface
    // gained a sentence rather than a feature.
    //
    // 1062 until `chat.sportTypesLabel` left with the control it named. That
    // picker let a coach author choose which sports their coach would see, and
    // the field behind it stopped filtering anything when the grounding window
    // was un-narrowed (2026-08-27) — so the label was captioning a promise the
    // product no longer made. A surface moved: the number moved with it.
    //
    // 1977 until the fuelling line reached the plan card. The ultra and heat
    // builder coaches had been attaching a `fueling_protocol` to every long
    // session since the structured-workout schema defined one — carbohydrate
    // g/h, fluid mL/h, an estimated sodium loss — and neither plan card had
    // anything to render it with, so the prescription was validated and then
    // dropped. Five keys, five locales, because a surface finally displays
    // something it had been sent all along.
    //
    // 1972 until the catalogue became one: the 237 server-rendered keys
    // (`messaging.*`, `commands.*`, `notifications.*`, `persona.*`) moved into these files
    // from the Rust table they used to live in, so the registry, the web app
    // and the phone read one source. A key counts once whichever side
    // renders it.
    //
    // 2334 until the memory predicate codes joined: 19 `messaging.memory.predicate.*`
    // keys, one sentence template per code, rendered on the server for the
    // memory screens, the recall tool and the coach dossier alike.
    //
    // 2352 after the French rendering sweep read the settings panes: two keys
    // for literals the scanner had walked past (a group panel description, the
    // phone's empty conversation list), minus three that translated the
    // coaching persona names while both clients render `PERSONA_NAME`
    // untranslated on purpose — the stored value is quoted in the coach prompt.
    //
    // 2352 until driving the app in French found what no gate could see: the
    // chat progress line ("generating response…") and the login divider
    // ("or") were English literals inside a shared function and a two-letter
    // text node. Six `chat.status.*` keys, `auth.orDivider`,
    // `tokens.activeCount` and `settingsUi.midnightUtc` — nine — replaced
    // them (carnet#206).
    //
    // Plus one for `/reset`'s catalogue description, which every command
    // listing renders from this corpus (carnet#189).
    const reference = leafKeys(bundleFor('en')).sort();
    expect(reference).toHaveLength(2362);

    for (const language of SUPPORTED_LANGUAGES) {
      expect(leafKeys(bundleFor(language)).sort()).toEqual(reference);
    }
  });

  it('translates the corpus rather than declaring it', () => {
    const english = bundleFor('en');
    const read = (bundle: Record<string, unknown>, key: string): string =>
      key.split('.').reduce<unknown>((node, part) => (node as Record<string, unknown>)[part], bundle) as string;
    const keys = leafKeys(english).filter((key) => !SHARED_ACROSS_LANGUAGES.has(key));

    for (const language of SUPPORTED_LANGUAGES) {
      if (language === 'en') {
        continue;
      }
      const bundle = bundleFor(language);
      const untranslated = keys.filter((key) => read(bundle, key) === read(english, key));
      // Cognates are a proportion of the corpus, not a fixed number of them:
      // "Admin", "Coach", "Notifications", "Version" are the same word in
      // several of these languages, and more keys means more of them. This
      // was an absolute 12, which was right at 178 keys and wrong at 400 —
      // a bound that has to be raised every time the corpus grows teaches
      // everyone to raise it. The highest real rate measured here is 5.5%
      // (French); a locale that never diverged would sit near 100%.
      const rate = untranslated.length / keys.length;
      expect(
        rate,
        `${language}: ${untranslated.length}/${keys.length} identical to English — ${untranslated.slice(0, 20).join(', ')}`,
      ).toBeLessThanOrEqual(0.08);
    }
  });
});
