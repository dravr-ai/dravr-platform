# @pierre/i18n

Unified internationalization for the Dravr web and mobile apps.

## The contract this package exists to keep

The platform has two locale systems and they used to disagree:

- the **server** owns *reply* language — `users.locale`, `DEFAULT_LOCALE = "fr"`, five
  locales enforced at `entries == keys * 5` in
  `crates/pierre-contremaitre/src/messaging_strings.rs`;
- the **clients** own *chrome* language — i18next, persisted per device.

A user could read the app in English while their coach answered in French, because
nothing joined the two. This package joins them: `initI18n` takes a **required**
`persistLocale` writer, and every language change made through
`useLanguageSwitcher` / `useLanguageSwitcherNative` writes both halves —
i18next for what the user reads, `PUT /api/user/locale` for what the coach answers in.

`SUPPORTED_LANGUAGES` is therefore exactly the server's `SUPPORTED_LOCALES`, and
`DEFAULT_LANGUAGE` is exactly `DEFAULT_LOCALE`:

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

initI18n({ persistLocale });
```

```ts
// frontend/src/i18n/localePersister.ts
import type { LocalePersister } from '@pierre/i18n';
import { userApi } from '../services/api';

export const persistLocale: LocalePersister = async (language) => {
  await userApi.updateLocale(language);
};
```

Mobile is the same call from `app/_layout.tsx`. Test runners initialize it too —
`frontend/src/test/setup.ts` and `frontend-mobile/jest.setup.js` — with a persister
that **rejects**, so a test that changes language has to register the writer it means
to assert instead of passing on a silent no-op.

## Using translations

```tsx
import { useTranslation } from '@pierre/i18n';

function Row() {
  const { t } = useTranslation();
  return <p>{t('settings.languageDescription')}</p>;
}
```

Keys are dot-notation over eight namespaces: `common`, `auth`, `chat`,
`settings`, `insights`, `providers`, `errors`, `validation`.
Interpolation uses `{{name}}`: `t('validation.minLength', { min: 8 })`.

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

1. add the locale to `SUPPORTED_LOCALES` in `crates/pierre-routes-auth/src/login.rs`;
2. add its column to `messaging_strings.rs` — all keys, or the invariant test reds;
3. create `src/locales/<tag>/translation.json` with **every** key translated;
4. add it to `SUPPORTED_LANGUAGES`, `LANGUAGE_NAMES` and `defaultI18nConfig.resources`;
5. add its flag to both `LanguageSwitcher` components.

`frontend/src/i18n/__tests__/localeCorpus.test.ts` fails on a locale that is declared
but short of keys, or that never diverged from English.

## License

MIT OR Apache-2.0
