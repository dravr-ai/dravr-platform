# @pierre/i18n

Unified internationalization for the Dravr web and mobile apps.

## The contract this package exists to keep

There is **one string catalogue**, and it is authored in
[dravr-contremaitre](https://github.com/dravr-ai/dravr-contremaitre), `strings/<locale>.json`.
The five nested files in `src/locales/<locale>/translation.json` are the platform's
**byte-identical copy of the pinned contremaitre rev**: the contremaitre bump lane
regenerates them with `scripts/ci/sync-contremaitre-fallback.sh` in the same commit as
the pin, and they are never edited by hand — `scripts/ci/check-contremaitre-sync.sh`
(pre-push Tier 1b) fails a push on any byte of difference. Everything a user reads comes
out of them, on every surface:

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
screen. Now a key exists once, in all five locales: contremaitre's own tests hold the
catalogue to that, and on this side Tier 1b requires an identical key set across the
five files and every `KEY_*` the registry declares to be present;
`crates/pierre-server/tests/contremaitre_test.rs` proves the same at compile time, and
`packages/i18n/__tests__/locale-corpus.test.ts` pins which file each offered locale reads.

**A key is rendered by exactly one side.** Server-rendered keys (`messaging.*`,
`commands.*`, `notifications.*`, `persona.*`, `hosted.*` — the ones with a `KEY_*` constant) use
positional `{0}`, `{1}` placeholders, filled by `format_template` in Rust. Client
keys use i18next's `{{name}}`. Tier 1b rejects a key that mixes them.

The second thing this package joins is the *language*: `initI18n` takes a
**required** `persistLocale` writer, and every language change made through
`useLanguageSwitcher` / `useLanguageSwitcherNative` writes both halves —
i18next for what the user reads, `PUT /api/user/locale` for what the agent
answers in — so the chrome and the agent never disagree. The same holds on
mount: a language remembered on this device wins over `serverLocale`, and when
a signed-in account's `users.locale` disagrees with it the restore writes the
device choice back, reporting a failed write through `syncState`. With nobody
signed in and nothing chosen on the device, the restore uses the last signed-in
account's locale, which it keeps under its own key
(`pierre_app_language_account`) so it never outranks or overwrites the next
account — that is what keeps the login screen in the account's language.

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

initI18n({ persistLocale, fetchBundle: i18nApi.bundle });
```

```ts
// frontend/src/i18n/localePersister.ts
import type { LocalePersister } from '@pierre/i18n';
import { userApi } from '../services/api';

export const persistLocale: LocalePersister = async (language) => {
  await userApi.updateLocale(language);
};
```

`fetchBundle` is the app's api-client `i18nApi.bundle`, passed straight from each app
root — it needs no wrapper. It is optional and fail-open: init renders
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

Add the key to **all five** `strings/<locale>.json` files in dravr-contremaitre, nested
under its namespace, and land it there. The platform receives it when the contremaitre
pin moves: the bump lane copies the files here and commits them with the pin. Never add
or edit a string in this package first — the byte-identity check fails the push, and the
next bump would revert it anyway. A string the server renders also gets a
`pub const KEY_*` in `crates/pierre-contremaitre/src` (`messaging_strings.rs`, or
`hosted_strings.rs` for the hosted connect pages) naming the dotted key, and uses `{0}`
placeholders; that constant lands in the platform after the pin bump that carries its key.

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

Adding one here without adding it to the server ships a language the agent cannot
answer in. The order is:

1. add the locale to `SUPPORTED_LOCALES` in `crates/pierre-core/src/models/user.rs`
   — the registry, `PUT /api/user/locale` and `GET /api/i18n/{locale}` all read it;
2. add `strings/<tag>.json` with **every** key translated to dravr-contremaitre, bump
   the pin, create `src/locales/<tag>/` so the sync script copies it (it refuses a
   locale on one side only), and embed it in `messaging_strings.rs` next to the other five;
3. add it to `SUPPORTED_LANGUAGES`, `LANGUAGE_NAMES` and `defaultI18nConfig.resources`;
4. add its flag to both `LanguageSwitcher` components.

`packages/i18n/__tests__/locale-corpus.test.ts` fails on a locale that is declared
but not bound to its own catalogue file.

## License

MIT OR Apache-2.0
