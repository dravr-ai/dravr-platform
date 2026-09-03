// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Makes the scanner fail on purpose, one syntactic shape at a time
// ABOUTME: A ceiling of 0 means nothing unless the scanner can SEE each shape it claims to

import fs from 'fs';
import os from 'os';
import path from 'path';
import { describe, it, expect, afterEach } from 'vitest';
import { scanUntranslated, isAthleteSurface } from '../untranslatedScan';

/**
 * The ratchet counts. A count cannot tell you what it failed to look at.
 *
 * Three times in one day the gate reported zero hardcoded strings and was
 * wrong — first because `ROOTS` covered two directories, then because
 * `accessibilityLabel` matched no pattern, then because argument prose only
 * matched in the first argument of a call. Each time the number looked like
 * evidence and was not.
 *
 * So the shapes are the thing under test: write a file containing exactly one
 * instance of each, and assert the scanner finds it. A pattern that stops
 * matching fails here rather than silently lowering the count.
 */
let tmpDir: string | null = null;

/** Write `source` into a throwaway athlete-surface file and scan just that. */
function scan(source: string): string[] {
  tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), 'scan-shapes-'));
  // `components/chat` is in ATHLETE_SURFACES, so the hits count as athlete scope.
  const dir = path.join(tmpDir, 'components', 'chat');
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, 'Probe.tsx');
  fs.writeFileSync(file, source);
  return scanUntranslated([dir]).map((h) => h.text);
}

afterEach(() => {
  if (tmpDir) {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    tmpDir = null;
  }
});

describe('the scanner sees every shape it claims to', () => {
  it('a JSX text node', () => {
    expect(scan('const A = () => <Text>Ask anything about your training.</Text>;')).toContain(
      'Ask anything about your training.',
    );
  });

  it('a JSX text node wrapped across source lines', () => {
    // The scanner collapses whitespace, so a paragraph written over three
    // indented lines is one string. Most onboarding copy is written this way.
    const hits = scan(`const A = () => (
      <Text>
        Connect a fitness service to get
        started.
      </Text>
    );`);
    expect(hits).toContain('Connect a fitness service to get started.');
  });

  it('a JSX text node that runs into an interpolation', () => {
    expect(scan('const A = () => <Text>Confidence {pct}%</Text>;')).toContain('Confidence');
  });

  it('accessibilityLabel — React Native spells aria-label this way', () => {
    // Its absence hid every screen-reader string in the mobile app.
    expect(scan('const A = () => <B accessibilityLabel="Back to chats" />;')).toContain(
      'Back to chats',
    );
  });

  it('helpText and the other prose props', () => {
    expect(scan('const A = () => <B helpText="This is the one your coach comes back to." />;'))
      .toContain('This is the one your coach comes back to.');
  });

  it('prose in the FIRST argument of a call', () => {
    expect(scan("function f() { setError('Login failed'); }")).toContain('Login failed');
  });

  it('prose in a LATER argument of a call', () => {
    // `Alert.alert(t('common.error'), 'Failed to revoke token')` read clean
    // while the pattern anchored only on `(`.
    expect(scan("function f() { alertx(t('common.error'), 'Failed to revoke token'); }")).toContain(
      'Failed to revoke token',
    );
  });

  it('a label field inside an object literal, parentheses and all', () => {
    // CODE_SHAPE rejects anything containing `(`; a value behind `text:` is
    // copy by construction, so the filter must not apply to it.
    expect(scan("const b = [{ text: 'Member (athlete)' }];")).toContain('Member (athlete)');
  });

  it('a template-literal sentence containing a question mark and quotes', () => {
    // Ten Alert bodies stayed English behind a character class that rejected
    // `?` and `"`, while their titles translated around them.
    const hits = scan('function f() { alertx(t("k"), `Leave "${name}" from the group?`); }');
    expect(hits.some((h) => h.includes('from the group'))).toBe(true);
  });

  it('both arms of a ternary', () => {
    const hits = scan("const s = busy ? 'Signing in now' : 'Sign in please';");
    expect(hits).toContain('Signing in now');
    expect(hits).toContain('Sign in please');
  });
});

describe('the scanner does NOT flag things that are not copy', () => {
  it('a description prop — the Discover header and both onboarding cards hid here', () => {
    expect(scan('<TabHeader description="Find AI coaching assistants" />')).toContain(
      'Find AI coaching assistants',
    );
    expect(
      scan('<ChoiceCard description="Track your own training and get coaching tuned to how you actually train." />'),
    ).toContain('Track your own training and get coaching tuned to how you actually train.');
  });

  it('prose assigned to a top-level constant — SLASH_HINT in a shared package', () => {
    expect(
      scan("export const SLASH_HINT = 'Type / for commands · @handle brings a coach in for one turn';"),
    ).toContain('Type / for commands · @handle brings a coach in for one turn');
    // A key table and an enum value are data, not copy.
    expect(scan("export const SPORT_LABEL_KEY = 'app.sportRunning';")).toEqual([]);
    expect(scan("const KIND = 'north_star';")).toEqual([]);
  });

  it('a font family', () => {
    expect(scan("const s = { fontFamily: 'Menlo' };")).not.toContain('Menlo');
  });

  it('an argument to .includes(), which is matched rather than shown', () => {
    // ErrorBoundary tests the browser's own "Loading chunk" text to detect a
    // stale bundle; translating it would disable the auto-reload.
    expect(scan("const b = e.message.includes('Loading chunk');")).not.toContain('Loading chunk');
  });

  it('console output', () => {
    expect(scan("function f() { console.error('Failed to open URL', e); }")).not.toContain(
      'Failed to open URL',
    );
  });

  it('a one-word element of a string array', () => {
    // The regression this test exists for: making `,` an argument opener also
    // made every second array element look like a call argument. At ceiling 0
    // that is a spurious push failure for whoever next writes one.
    const hits = scan("const TABS = ['Overview', 'Details'];");
    expect(hits).not.toContain('Details');
    expect(hits).not.toContain('Overview');
  });

  it('a comment', () => {
    expect(scan('// Something went wrong here\nconst a = 1;')).not.toContain(
      'Something went wrong here',
    );
  });
});

/**
 * The scope itself, pinned in both directions.
 *
 * `isAthleteSurface` is derived now — a walk from the entry points an athlete
 * enters through, not a hand-maintained list of components. That fixes a list
 * that had rotted 175 strings behind the app, and introduces a failure the list
 * could not have: a derived scope can SHRINK SILENTLY. Miss a barrel or a lazy
 * import and those files leave athlete scope, the ceiling of 0 goes on passing,
 * and the gate reports success while measuring less.
 *
 * Every one of this ratchet's three failures in a single day had that shape —
 * the number looked like evidence and was not. So the classification is
 * asserted for files whose answer we know, in both directions: a walk that
 * under-includes fails here, and so does one that over-includes until the
 * operator console vanishes into athlete scope and stops being anybody's
 * decision.
 *
 * Not an exception list. Those excuse files from a rule and accumulate; this
 * asserts a rule holds, and a file changing side is a bug rather than something
 * to be pended.
 */
describe('athlete scope is derived, and pinned where we know the answer', () => {
  const web = path.join(__dirname, '../..');
  const mobile = path.join(__dirname, '../../../../frontend-mobile/src');

  it.each([
    // Reached only through `UserSettings` — the case the old list got wrong.
    // Its mobile twin was translated while this stayed English, so one athlete
    // read French messaging settings on the phone and English in the browser.
    ['components/MessagingSettingsTab.tsx', 'one hop under UserSettings'],
    // The GDPR disclosure that started the audit.
    ['components/PrivacySettingsTab.tsx', 'a privacy promise an athlete reads'],
    // Reached through a barrel, not a direct file import.
    ['components/ui/Card.tsx', 'reached through components/ui/index.ts'],
    // Mounted as `lazy(() => import('./notifications/NotificationsPanel'))`.
    ['components/notifications/NotificationsPanel.tsx', 'a lazy-imported route'],
  ])('%s is athlete — %s', (relative) => {
    expect(isAthleteSurface(path.join(web, relative))).toBe(true);
  });

  it.each([
    // Admin-only: `activeTab === 'connections'` is in ADMIN_ONLY_TABS, while an
    // athlete reaches Data Providers through UserSettings instead. Judged
    // athlete by its name during the audit and it is not — the walk was right
    // and the reading was wrong.
    ['components/UnifiedConnections.tsx', "'connections' is an ADMIN_ONLY_TAB"],
    ['components/EvalHarnessTab.tsx', 'the eval harness is operator chrome'],
    ['components/UserDetailDrawer.tsx', 'user administration'],
  ])('%s is operator — %s', (relative) => {
    expect(isAthleteSurface(path.join(web, relative))).toBe(false);
  });

  it('a shared package is athlete when an athlete entry point imports it', () => {
    // `ChatTab` imports `@pierre/shared-constants`; the walk follows the bare
    // specifier into the package, so its label tables are judged like any
    // component an athlete reaches. The generated capability catalogue is a
    // barrel re-export away and must be reached the same way.
    const packages = path.join(web, '../../packages');
    expect(isAthleteSurface(path.join(packages, 'shared-constants/src/commands.ts'))).toBe(true);
    expect(isAthleteSurface(path.join(packages, 'shared-constants/src/onboarding.ts'))).toBe(true);
    expect(isAthleteSurface(path.join(packages, 'shared-types/src/verdict.ts'))).toBe(true);
  });

  it('every file in the mobile app is athlete, including outside screens/', () => {
    // There is no operator console in the mobile app, so no file in it can be
    // operator chrome. Fifteen `screens/*` fragments used to stand in for that
    // rule and anything outside them fell through — which is how the offline
    // banner shipped in English to an app whose default locale is French.
    expect(isAthleteSurface(path.join(mobile, 'components/ServerStatusBanner.tsx'))).toBe(true);
    expect(isAthleteSurface(path.join(mobile, 'providers/QueryProvider.tsx'))).toBe(true);
    expect(isAthleteSurface(path.join(mobile, 'screens/chat/ChatScreen.tsx'))).toBe(true);
  });

  it('does not swallow the whole console into athlete scope', () => {
    // The mirror failure of shrinking: a walk that over-includes makes the
    // ceiling of 0 pass vacuously and dissolves the 663 English-by-decision
    // strings into a number nobody has to answer for.
    const hits = scanUntranslated([path.join(web, 'components')]);
    const operator = hits.filter((h) => h.scope === 'operator');
    expect(operator.length).toBeGreaterThan(0);
  });

  it('no import specifier is computed, because the walk could not follow one', () => {
    // The walk resolves string literals. Every dynamic import in both apps is
    // one today, so this boundary has no surface — and this test is what keeps
    // it that way: add `import(someVariable)` and the walk stops seeing whatever
    // is behind it, silently. Failing here is the warning.
    const roots = [web, path.join(web, '../../frontend-mobile/src')];
    const offenders: string[] = [];
    const walk = (dir: string): void => {
      for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          if (entry.name !== '__tests__' && entry.name !== 'node_modules') {
            walk(full);
          }
        } else if (/\.tsx?$/.test(entry.name) && !/\.test\.tsx?$/.test(entry.name)) {
          const source = fs.readFileSync(full, 'utf-8');
          for (const m of source.matchAll(/import\(\s*([^'"\s)])/g)) {
            offenders.push(`${path.relative(web, full)}: import(${m[1]}…`);
          }
        }
      }
    };
    for (const root of roots) {
      if (fs.existsSync(root)) {
        walk(root);
      }
    }
    expect(offenders).toEqual([]);
  });
});

/**
 * A proper noun under a copy key, which shape cannot tell from copy.
 *
 * `name: 'Telegram'` and `name: 'Cancel'` are the same shape: one capitalised
 * word behind a key that carries copy elsewhere. Every Dashboard tab is
 * `name: t('shell.navUsers')`, so `name` cannot be dropped from the pattern
 * without blinding the scanner to the tab strip — which is the exact miss this
 * scanner was built for. Its opening comment records it: the first sweep
 * "reported a file clean that still had 44 strings in it — `Cancel` is one
 * word, and an entire tab strip was declared in an object literal".
 *
 * So the flag stays, and the remedy is a brand constant rather than a
 * translation. Four channel names were flagged this way and moved to
 * `CHANNEL_BRAND` instead of being keyed, which is the right outcome: they are
 * identifiers now and no filter needs an opinion about them.
 */
describe('a proper noun is flagged, and a brand constant is the answer', () => {
  it('flags a trademark behind a copy key, because it cannot know', () => {
    expect(scan("const c = { name: 'Telegram' };")).toContain('Telegram');
  });

  it('flags one-word copy behind the same key, which is why it must', () => {
    expect(scan("const c = { name: 'Cancel' };")).toContain('Cancel');
  });

  it('lets a CamelCase name through, as the identifier heuristic requires', () => {
    // `WhatsApp` escapes on NOT_PROSE's `^[A-Z][a-z]+[A-Z]` — the rule that
    // keeps `SomeComponent` out. So the two halves of one brand table are
    // treated differently, and that asymmetry is the price of not flagging
    // every identifier in the codebase. Pinned so it is a known boundary
    // rather than a surprise at ceiling 0.
    expect(scan("const c = { name: 'WhatsApp' };")).not.toContain('WhatsApp');
  });
});

/**
 * A paragraph long enough to fall off the end of the text pattern.
 *
 * `JSX_TEXT` bounds its capture, and the bound counts raw source — the
 * indentation and newlines of wrapped JSX, not the sentence a reader sees. At
 * 200 that hid seven strings, three of them under 200 characters of actual
 * copy. They were not filtered as non-copy; they were never matched, which is
 * the failure that makes a ceiling of 0 mean nothing.
 *
 * The bound is exactly the kind of constant someone tightens later for
 * performance, so this is the test that argues back.
 */
describe('prose longer than the text pattern bound', () => {
  it('finds a paragraph whose raw span exceeds 200 characters', () => {
    // Wrapped and indented the way real JSX is, so the raw span is what grows.
    const paragraph =
      'Help improve Dravr by sharing anonymized usage data. We track general ' +
      'usage patterns like which tools you reach for and how often you train, ' +
      'never the content of your conversations and never your fitness data ' +
      'itself, and every identifier is hashed before it leaves the device.';
    const source = `<div>\n      <p>\n        ${paragraph}\n      </p>\n    </div>`;
    expect(paragraph.length).toBeGreaterThan(200);
    expect(scan(source)).toContain(paragraph);
  });

  it('still finds a sentence whose COPY is short but whose source span is not', () => {
    // The case neither of us predicted, and the one the old bound hid most of:
    // 180 visible characters wrapped across indented lines is well over 200 raw.
    // The raw span is asserted rather than assumed — a first draft of this test
    // built a span of ~152 characters, passed at the old bound, and proved
    // nothing at all.
    const sentence =
      'Facts the platform extracted from your conversations so the coach keeps ' +
      'memory across sessions, which you can review and remove at any time from ' +
      'this screen whenever you want to.';
    const wrapped = sentence.split(' ').reduce<string[]>((lines, word) => {
      const last = lines[lines.length - 1];
      if (last !== undefined && `${last} ${word}`.length < 40) {
        lines[lines.length - 1] = `${last} ${word}`;
      } else {
        lines.push(word);
      }
      return lines;
    }, []);
    const indent = '\n            ';
    const span = wrapped.join(indent);

    expect(sentence.length).toBeLessThan(200);
    // The bound counts THIS, which is the whole point.
    expect(span.length).toBeGreaterThan(200);

    expect(scan(`<div>\n          <p>${indent}${span}\n          </p>\n        </div>`)).toContain(
      sentence,
    );
  });

  it('finds copy a function hands back, the shape the chat progress line hid in', () => {
    // `packages/chat-utils/src/progress.ts` mapped every turn-progress event to
    // English this way and both chat UIs rendered it verbatim, while the file
    // was inside the scan and no shape matched a `return` (carnet#206).
    const source = [
      'export function statusText(stage: string): string {',
      "  if (stage === 'dispatch') return 'generating response…';",
      "  return 'running a tool…';",
      '}',
    ].join('\n');
    const hits = scan(source);
    expect(hits).toContain('generating response…');
    expect(hits).toContain('running a tool…');
  });

  it('leaves an interpolated return to the render sweep, not to a regex', () => {
    // `return \`calling ${tool}…\`` is copy, but NOT_PROSE rejects a brace on
    // purpose: unpicking that flags every enum and brand constant in the app
    // (the WhatsApp case below). The rendered line is caught instead — the
    // French sweep fails when a page paints an English value the corpus
    // translates (carnet#206).
    const source = [
      'export function calling(tool: string): string {',
      '  return `calling ${tool} now…`;',
      '}',
    ].join('\n');
    expect(scan(source)).toEqual([]);
  });

  it('leaves a returned Tailwind class list alone', () => {
    // The verdict chips and the A2A badges choose styling with exactly the
    // shape above. Flagging those buries the real copy in noise: sixteen of
    // the eighteen first hits were class lists.
    const source = [
      'export function chipClass(kind: string): string {',
      "  if (kind === 'supported') return 'bg-success/15 text-on-success-container';",
      "  return 'bg-surface-container-high text-on-surface';",
      '}',
    ].join('\n');
    expect(scan(source)).toEqual([]);
  });

  it('finds a two-letter text node, the shape the login divider hid in', () => {
    // The divider between the password form and the Google button is the
    // shortest real copy either app renders, and a three-character floor could
    // not see it — in the text pattern or in the length filter after it. The
    // provider modal spells it `Or`; the login page spells it `or`.
    //
    // Only the capitalised one is caught here. A bare lowercase pair is a unit
    // as often as a word (`km`, `kg`), and an all-caps one reads as a constant
    // name to NOT_PROSE, so both are left to rendering: `auth.orDivider` is a
    // corpus key, and the French sweep fails when a page paints the English
    // value (carnet#206).
    expect(scan('<div>\n  <span className="uppercase">Or</span>\n</div>')).toContain('Or');
  });

  it('finds double-quoted prose assigned to a constant', () => {
    // Every pattern that accepts two quote styles captures into two groups,
    // and the reader took only the first — so a double-quoted constant matched
    // and was then dropped on the floor.
    expect(scan('const SLASH_HINT = "Type / for commands";')).toContain(
      'Type / for commands',
    );
  });
});
