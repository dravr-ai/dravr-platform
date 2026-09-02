# @pierre/i18n

Unified internationalization for the Dravr web and mobile apps.

## The contract this package exists to keep

There is **one string catalogue**: the five nested files in `src/locales/<locale>/translation.json`.
Everything a user reads comes out of them, on every surface:

- the **server** embeds them at build time (`include_str!` in
  `crates/pierre-contremaitre/src/messaging_strings.rs`) and seeds the
  messaging-strings registry from them — the registry renders every Telegram,
  WhatsApp, Slack and `/help` reply, and contremaitre overlays it at runtime;
- the **clients** embed the same files (this package's `defaultI18nConfig`) and
  overlay the registry's live copy through `GET /api/i18n/{locale}` at start-up
  and on every language change, so a string fixed upstream reaches a phone on
  its next open without a store release.

The same catalogue used to exist twice — a Rust table of 213 messaging keys and a
JSON corpus of ~2000 chrome keys, each with its own five-locale gate — and a
sentence could be French in the chat and English in the onboarding wizard on one
screen. Now a key exists once, in all five locales, or the push fails:
`scripts/ci/check-contremaitre-sync.sh` (pre-push Tier 1b, compile-free) requires
an identical key set across the five files and every `KEY_*` the registry declares
to be present; `crates/pierre-server/tests/contremaitre_test.rs` proves the same
at compile time; `frontend/src/i18n/__tests__/localeCorpus.test.ts` pins the count.

**A key is rendered by exactly one side.** Server-rendered keys (`messaging.*`,
`commands.*`, `notifications.*`, `persona.*` — the ones with a `KEY_*` constant) use
positional `{0}`, `{1}` placeholders, filled by `format_template` in Rust. Client
keys use i18next's `{{name}}`. Tier 1b rejects a key that mixes them.

The second thing this package joins is the *language*: `initI18n` takes a
**required** `persistLocale` writer, and every language change made through
`useLanguageSwitcher` / `useLanguageSwitcherNative` writes both halves —
i18next for what the user reads, `PUT /api/user/locale` for what the coach
answers in — so the chrome and the coach never disagree.

`SUPPORTED_LANGUAGES` is therefore exactly the server's `SUPPORTED_LOCALES`
(`pierre_core::models`), and `DEFAULT_LANGUAGE` is exactly `DEFAULT_LOCALE`:

| Locale | Name | |
|---|---|---|
| `fr` | Français | default, matching `DEFAULT_LOCALE` |
| `en` | English | |
| `es` | Español | |
| `de` | Deutsch | |
| `pt` | Português | European Portuguese, `tu` form |

## Entry points

| Import | Contents |
|---|---|
| `@pierre/i18n` | everything platform-neutral, plus `useLanguageSwitcher` (localStorage) |
| `@pierre/i18n/native` | `useLanguageSwitcherNative` (AsyncStorage) |

The native hook lives behind a subpath so a web bundle never pulls React Native in.
Mobile resolves both through `metro.config.js` (`resolveRequest`), `tsconfig.json`
(`paths`) and `jest.config.js` (`moduleNameMapper`).

## Setup

Each app registers its own writer once, at the root, before the first render.

```tsx
// frontend/src/main.tsx
import { initI18n } from '@pierre/i18n';
import { persistLocale } from './i18n/localePersister';
import { fetchBundle } from './i18n/fetchBundle';

initI18n({ persistLocale, fetchBundle });
```

```ts
// frontend/src/i18n/localePersister.ts
import type { LocalePersister } from '@pierre/i18n';
import { userApi } from '../services/api';

export const persistLocale: LocalePersister = async (language) => {
  await userApi.updateLocale(language);
};
```

`fetchBundle` is the app's api-client `i18n.bundle` (`frontend/src/i18n/fetchBundle.ts`,
`frontend-mobile/src/i18n/fetchBundle.ts`). It is optional and fail-open: init renders
the embedded catalogue synchronously, the live overlay lands afterwards through
`addResourceBundle` (mounted chrome repaints via `bindI18nStore: 'added'`), and a
fetch that fails changes nothing on screen. The digest of each bundle applied goes
back as `If-None-Match`, so an unchanged catalogue is a bodiless 304.

Mobile is the same call from `app/_layout.tsx`. Test runners initialize it too —
`frontend/src/test/setup.ts` and `frontend-mobile/jest.setup.js` — with a persister
that **rejects** and no `fetchBundle`, so a test that changes language has to register
the writer it means to assert instead of passing on a silent no-op, and no test ever
touches the network for strings.

## Using translations

```tsx
import { useTranslation } from '@pierre/i18n';

function Row() {
  const { t } = useTranslation();
  return <p>{t('settings.languageDescription')}</p>;
}
```

Keys are dot-notation over the catalogue's namespaces (`common`, `auth`, `chat`,
`onboarding`, `settings`, `providers`, `errors`, `validation`, …).
Interpolation uses `{{name}}`: `t('validation.minLength', { min: 8 })`.

## Adding a string

Add the key to **all five** `src/locales/<locale>/translation.json` files, nested
under its namespace, and nothing else. A string the server renders also gets a
`pub const KEY_*` in `messaging_strings.rs` naming the dotted key, and uses `{0}`
placeholders. Tier 1b fails the push on a key that is short of a locale, and
`localeCorpus.test.ts` needs its count bumped — that bump is the review prompt.

A string is never written into a component, a constants package or a Rust
literal: `frontend/src/i18n/untranslatedScan.ts` ratchets the athlete surface at
zero hardcoded strings, and a label table in `@pierre/shared-constants` holds a
key, resolved through `t()` at render.

## Switching language

```tsx
import { useLanguageSwitcher, SUPPORTED_LANGUAGES, LANGUAGE_NAMES } from '@pierre/i18n';

const { currentLanguage, changeLanguage, syncState } = useLanguageSwitcher({
  serverLocale: user?.locale,
});
```

- `serverLocale` is adopted on first load **only** when this device has no stored
  choice, so a language picked on the web carries over to the phone.
- `changeLanguage` never rejects. `syncState` reports the server half:
  `'saving'` while the PUT is in flight, `'error'` once the chrome moved but
  `users.locale` did not. Render that error — a silently dropped write is the
  disagreement this package exists to close.

Both `LanguageSwitcher` components (`frontend/src/components/LanguageSwitcher.tsx`,
`frontend-mobile/src/components/LanguageSwitcher.tsx`) already do this, and are
mounted in the web `UserSettings` Appearance card and the mobile `SettingsScreen`
language section.

## Adding a locale

Adding one here without adding it to the server ships a language the coach cannot
answer in. The order is:

1. add the locale to `SUPPORTED_LOCALES` in `crates/pierre-core/src/models/user.rs`
   — the registry, `PUT /api/user/locale` and `GET /api/i18n/{locale}` all read it;
2. create `src/locales/<tag>/translation.json` with **every** key translated, and
   embed it in `messaging_strings.rs` next to the other five;
3. add it to `SUPPORTED_LANGUAGES`, `LANGUAGE_NAMES` and `defaultI18nConfig.resources`;
4. add its flag to both `LanguageSwitcher` components.

`frontend/src/i18n/__tests__/localeCorpus.test.ts` fails on a locale that is declared
but short of keys, or that never diverged from English.

## License

MIT OR Apache-2.0
