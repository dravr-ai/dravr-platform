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
    // coaching persona names while both clients render the brand name
    // untranslated on purpose — the stored value is quoted in the coach
    // prompt. (That name is `display_name` on the server's card now.)
    //
    // 2352 until driving the app in French found what no gate could see: the
    // chat progress line ("generating response…") and the login divider
    // ("or") were English literals inside a shared function and a two-letter
    // text node. Six `chat.status.*` keys, `auth.orDivider`,
    // `tokens.activeCount` and `settingsUi.midnightUtc` — nine — replaced
    // them (carnet#206).
    //
    // 2371 since /fortnight added ten command strings; 2361 until the scan
    // reached `.ts` and found the quota banner building
    // three English sentences by hand on both clients. Seven `usage.*` keys
    // carry them, counter labels included, and the notification and A2A
    // relative times became `Intl.RelativeTimeFormat` rather than more keys
    // (carnet#207).
    //
    // 2369 until the mobile half of that debt was paid: the voice-input
    // vocabulary, the provider-connection alerts, the conversation and quota
    // failures and the Google sign-in refusals — the hooks return keys now,
    // and the ratchet's athlete ceiling is 0 again with the whole `.ts` half
    // of both apps inside it.
    //
    // 2402 until the phone was driven in French: the hero's second line and
    // one of the four pillar words were hardcoded English beside three
    // translated ones, and every auth validation message was English. Nine
    // keys — `app.activity` and eight `validation.*` — and two existing keys
    // wired up. Every one of those shapes is invisible to the scanner (text
    // after an expression, a bare string in an array, an assignment to an
    // object property), which is why the phone needs a render sweep.
    //
    // 2411 until `Intl.RelativeTimeFormat` turned out not to exist on the
    // phone's JavaScript engine and crashed the notification centre. Four
    // `notifications.*` keys carry the relative times instead: the catalogue
    // is on every runtime, and these four phrasings need no plural rule
    // (carnet#227).
    //
    // 2414 until the memory screen stopped carrying two vocabularies. Its
    // title, blurb and both empty-state lines existed twice — once under
    // `shell.*` for the browser, once under `app.*` for the phone — and the
    // copies had already drifted apart by a possessive and a rewrite. The four
    // `app.*` duplicates are gone and both clients read the survivors, which is
    // -4; the filtered-empty state the web panel was missing, the connected-app
    // screen-reader label and the two states a messaging list can be in that
    // are not "everything is linked" are +6.
    //
    // 2389 until the verdict drawer stopped scrubbing its own subline. It
    // borrowed the chip's "{{count}} verdicts · {{qualifier}}" with an empty
    // qualifier and stripped the dangling separator back off with a regex, so
    // a locale that punctuated the chip differently kept its separator.
    // `chat.verdictsCount` says the one thing the subline means.
    //
    // 2416 until the persona cards became the server's. Both clients held
    // four hand-written options — a tagline, a blurb and up to two bullets
    // each — describing contracts they could not see, while
    // `GET /api/personas` rendered the same cards from the live contract
    // registry and nobody read it. Eighteen `app.style*` keys left with them.
    //
    // 2380 until the backfill notice learned to reach the app. A completed
    // historical backfill is delivered into a web or mobile conversation as a
    // persisted turn rather than through a channel adapter, and the app push
    // that announces it needs a title and a one-line body of its own —
    // `messaging.backfill.push_title` and `.push_body`, +2.
    //
    // 2382 until the quota notice stopped saying one thing at two levels.
    // `messaging.quota_warning` renders "{used} of {limit}", which is fine at
    // 45 of 50 and nonsense at 670828 of 500000 — an athlete read that four
    // turns running on 2026-09-02. `messaging.quota_burst` is the sentence for
    // past the cap: it names the limit and the reset and never prints the
    // comparison. One key, five locales, because a surface gained a state it
    // already had and could not describe (carnet#251).
    //
    // 2383 until the Boreal v2 emoji sweep. The A2A client list's "shown once"
    // notice opened with a warning emoji, which the untranslated scanner read
    // as decoration and skipped; without it the sentence was a bare English
    // literal on screen. `a2a.secretShownOnce` is the key it always needed, +1.
    //
    // 2384 until the login copy stopped naming a persona at the athlete. The
    // phone's login carried `app.heroPersona` — "The Technical Naturalist" —
    // as a tracked over-line above the headline: internal vocabulary, in the
    // caps the type scale had already retired, on the first screen anyone
    // sees. The rewritten hero says what the product does instead, and the
    // key lost its only caller, so it is deleted rather than translated five
    // ways for nobody, -1.
    //
    // 2383 until the same sweep caught its two siblings. `app.heroLead` was a
    // byte-identical twin of `auth.taglineLead` in all five locales — the
    // phone had its own copy of the login headline, so editing the web pair
    // silently desynced it; the phone now reads the shared key. And
    // `auth.taglinePersona` is the other half of the retired brand line, an
    // orphan on exactly the grounds `heroPersona` was deleted for. -2.
    //
    // 2381 until the coach→agent rename (carnet#346). Sixty-eight keys that
    // still spelled the old vocabulary had no caller on any surface — the
    // retired `chat.*` coach-list chrome, the `discover.*` import dialog, the
    // `convPanel.*` grouping the conversation list dropped, the `groups.*`
    // landing copy — and were deleted rather than translated. Two pairs said
    // the same thing under two keys and became one: the human-coach "Remove
    // Coach" title (`app.*` on the phone, `groups.*` on the web) and the "I
    // coach others" onboarding card. Every remaining sense-B key — the human
    // professional, never the AI persona — now lives under `humanCoach.*`.
    // The count is read off the tree, never carried: it was 2383, 2384 and
    // 2381 on three different afternoons, each correct when taken. -68.
    // +7 for the route block: chat.routeAlt, routeAltTitled, routeClimbs,
    // routeClimbCategory, routeNoTrack and the two map-attribution strings.
    // +10 for the season walk: the eight `commands.season.*` strings and the
    // two horizon predicate sentences, `aim_this_season` and `aim_long_term`.
    // +9 for the flavours' plain-words labels, `messaging.flavour.*` — one per
    // catalogue flavour, what the agent says instead of "polarized".
    // +1 for `commands.help.domain.agent`: the agent commands used to be
    // grouped under the human coach's `/help` heading, so the identifier
    // rename gave them a heading of their own rather than renaming his.
    // +20 net for the plan card, `plan.card.*`: the season timeline and the
    // fortnight the card renders from the saved plan (28 keys), less the
    // eight the old builder-document card alone used (zones, TSS, CTL target,
    // polarization, easy floor, gut training, week, % of threshold).
    // +1 for `plan.card.alsoRacing`: the B and C races the athlete named,
    // which the prompt block already carried and the card now shows too.
    // 2371 until the messaging-connect screen stopped carrying two
    // vocabularies, neither of which said a whole sentence. Both clients
    // rendered a translated fragment followed by a hardcoded English tail —
    // "…{{channel}}. Then press Start to finish.", "…{{channel}}. You'll come
    // back here automatically once it's done.", " link…", " connection just
    // now." — so four of five locales read half-English on the screen that
    // decides whether an athlete ever reaches their coach. The web half held
    // its own `frag.*` copies of sentences `app.*` already said in full, and
    // the copies had drifted (`frag.couldntStartThe` vs
    // `app.couldNotStartConnection`). Six orphaned `frag.*` fragments are
    // deleted and both clients read the `app.*` survivors, -6;
    // `app.scanQrOtherDevice` and `app.tapBelowToConnect` became whole
    // sentences under names that say so, +0; `app.connectChannelTitle`
    // replaces the `Connect ${displayName}` template literal that was English
    // in all five locales at three call sites, +1. Net -5. Then 2365: the
    // phone's composer lost its `/` button (Boreal v2.2 Phase 3, the left slot
    // stays empty), and `app.composerCommandsAria`, the button's only string,
    // went with it, -1. Then 2366: the phone's connections list names a
    // provider whose token lapsed (Boreal v2.2 Phase 4), `providers.expired`,
    // +1. Then 2362: the chat's provider picker, which nothing could open,
    // was deleted with the four strings only it read (`app.connectAProvider`,
    // `app.connectedCheck`, `app.connectFirstBlurb`, `app.connectingProvider`),
    // -4.
    // 2325 since Boreal v2.2 Phase 5 (Discover, onboarding and groups) paid
    // down three more `app.*`/domain-namespace duplications the same way
    // Phase 4 paid down `providers.expired`'s siblings: the phone's Discover
    // screens dropped 21 `app.*` keys for their byte-identical `discover.*`
    // twins (`app.installAgent`, `app.sortBy`, `app.tags`… — the row/detail
    // page/editor restyle touched every call site anyway) and gained two,
    // `discover.install` (the row's bare action word — `app.installAgent`
    // read "Install Agent," too long for a 64pt row) and
    // `discover.postInstallTitle` (a hardcoded, untranslated English string
    // found mid-migration in `PostInstallHint`, now a real key in all five
    // locales); onboarding dropped 13 `app.ob*` flat keys for the newer
    // `onboarding.*` namespace web already reads (`app.obTellMeTitle`,
    // `obSportQuestion`, `obAnalyzing`… — the eight name-interpolated
    // `obGreeting`/`obHint` siblings stayed, since web still reads them and
    // no `onboarding.*` twin exists for their wording); groups dropped 5
    // `app.*` keys (`thisWeek`, `weeklyReport`, `concerns`, `highlights`,
    // `recommendations`) for the `groups.*` twins that already existed and
    // needed no new key. -39, +2, net -37.
    // 2326 once `/reset` counted against the conversation cap like the "+"
    // button: `commands.reset.quota` is the refusal the athlete hears on
    // every surface, with the cap interpolated, in place of the error's wire
    // text. +1.
    // 2328 once a failed turn's words left the transport: `chat.turnIdleAborted`
    // is the note for a turn the idle stop dropped, and `chat.turnTryAgain` the
    // suffix a mobile failure row carries, both read by the clients in the
    // athlete's language instead of English literals. +2.
    // 2330 once the Strava seat reclaimer warns an athlete before giving their
    // seat to someone else: `notifications.event.seat_release_warning` title
    // and body, rendered on the server into the notification feed, the push
    // and the linked chat channels. +2 (carnet#505).
    // 2347 once the group weekly digest stopped reaching Telegram as one
    // English line naming the group by its raw chat id:
    // `notifications.group_digest.*` is its title, summary, trend, and the
    // per-member volume, fresh-form and concern lines, rendered on the server
    // in each manager's language. +17.
    // 2348 once the digest is posted into the group's own chat and names only
    // the members who share their training: `notifications.group_digest.room_scope`
    // says how many of the group that is and how to join in. +1 (carnet#518).
    // 2361 once the group health flags and the weekly report stopped carrying
    // English sentences from the server: `groups.flagDetail.*` phrases each
    // flag's evidence (+5), `groups.report.*` the report's summary, fresh-form,
    // concern, review and trend lines (+6), and `groups.ofTotal` and
    // `groups.respondMentionedHint` replace two English literals in the same
    // Group info panel (+2), and `groups.report.trendStable` so a steady
    // week still says which way the group went (+1). The same panel's member
    // count, "(you)" and invite-use lines (`groups.memberCount`, `youSuffix`,
    // `inviteUses`, `inviteUsed`) replace English literals (+4). +18 (carnet#520).
    // 2380 once each group chose where its weekly digest goes, off by default:
    // `/group digest` renders its usage, the status of each of the three modes
    // (which `/group status` reuses), its refusal and the note for a plan
    // without the digest on the server (+6), plus its catalogue description
    // (+1); the web select and the phone's radio rows read the label, the
    // three mode names and a hint for each (+7). +14 (carnet#541).
    // 2361 once TrainingPeaks joined as a provider: its name, account and
    // username labels on web and mobile, its settings blurb, the connected
    // copy that stopped being an English literal, `common.username`, and
    // `providers.trainingpeaksNotice.*` — the exposure notice every connect
    // surface shows before the credentials, and the short line the mobile
    // onboarding row carries. +15.
    // 2363 once a provider could read the workouts its calendar plans:
    // `providers.scope.planned_workouts` is the word the connect card shows
    // for the capability TrainingPeaks declares. +1.
    // 2373 once a coach could read a member's TrainingPeaks workouts through
    // their own account: `notifications.event.delegation_*` titles and bodies
    // ask the member, tell the coach the answer, and tell both sides when
    // TrainingPeaks drops the athlete from the coach's roster, rendered on the
    // server into the feed, the push and the linked chat channels. +10.
    // 2420 once the group info could hold those links: `delegation.*` is the
    // coach's roster section, the member's request and its answers, and each
    // refusal the link routes send; `providers.connectedThrough` and
    // `pendingLink` are what the TrainingPeaks row says of a delegated
    // connection; `humanCoach.trainingpeaksAccount` and its hint badge a coach
    // account; `humanCoach.youCoach` is the coach's own badge on the group.
    // +47.
    // 2453 once main's digest keys (2380) met the TrainingPeaks branch's
    // +73 above, whose running totals were counted from an older base.
    // 2462 once COROS became a provider: its name, account title and e-mail
    // placeholder on the web login and the phone's, the settings blurb, and
    // the phone's connections and onboarding lines. +9.
    // 2465 once COROS carried its own exposure notice, as TrainingPeaks does:
    // `providers.corosNotice` title, body and acceptance. +3.
    // 2467 once a reply the provider cut off or filtered carried a caveat:
    // `messaging.reply_stop.truncated` and `.filtered`. +2.
    // 2466 once the legacy Fitbit Web API provider was deleted:
    // `providers.fitbit`, a brand name nothing rendered, went with it. -1.
    const reference = leafKeys(bundleFor('en')).sort();
    expect(reference).toHaveLength(2466);

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
