// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Finds user-facing strings still hardcoded in components instead of read from the corpus
// ABOUTME: One implementation, so the gate and any progress count can never disagree

import fs from 'fs';
import path from 'path';

/**
 * Where prose hides in a component.
 *
 * The first sweep of this codebase looked only at JSX text nodes of two words
 * or more. It reported a file clean that still had 44 strings in it — `Cancel`
 * is one word, and an entire tab strip was declared in an object literal where
 * no text node exists. Each pattern below is a place that sweep could not see.
 */
/**
 * A text node. The capital-letter anchor this used to carry missed prose that
 * continues across markup — the login heading reads "…rendered in ink." on the
 * far side of a `<br />`, lowercase, and stayed English through three passes
 * because of it. Lowercase captures are admitted only when they are more than
 * one word, which keeps bare identifiers out.
 *
 * It closes on `{` as well as `<`, because copy is regularly followed straight
 * by an expression — `Your coaches{isLoading ? '' : ` (${n})`}` read as no text
 * node at all while it only closed on a tag.
 */
// The upper bound counts RAW SOURCE, not the collapsed sentence — indentation
// and newlines included. Three of the seven strings a cap of 200 was hiding
// were under 200 characters of actual copy; they lost on whitespace, wrapped
// across five indented lines. So the cap silently drops prose well short of
// its own number, and it drops it by never matching at all rather than by
// filtering it as non-copy — the ceiling then reads 0 by measuring less.
//
// Among the seven was the sentence in PrivacySettingsTab naming what data
// actually leaves the device, on the one surface whose own comment insists
// both clients must promise the same thing. The longest real string in either
// app is 379 raw characters.
const JSX_TEXT = /(?<!=)>\s*([A-Za-z][^<>{}]{2,800}?)\s*[<{]/g;
/**
 * A quoted prop that carries prose.
 *
 * `accessibilityLabel` and `accessibilityHint` are React Native's spelling of
 * `aria-label`, and their absence here is why the entire mobile app's screen
 * reader vocabulary sat in English behind a gate reporting zero. They are not
 * covered by the lowercase `label` alternative — the capital L in
 * `accessibilityLabel` does not match it.
 */
const STRING_PROP =
  /\b(?:placeholder|aria-label|accessibilityLabel|accessibilityHint|title|description|alt|label|helpText|subtitle|hint|emptyText|submitText|cancelText|confirmLabel|cancelLabel)="([A-Z][^"]{2,120})"/g;

/**
 * Prose assigned to a top-level constant.
 *
 * `export const SLASH_HINT = 'Type / for commands · …'` in a shared package
 * was read verbatim by both clients and never seen by any pattern above: not
 * a text node, not a prop, not a call argument. The `description` prop is the
 * same story — `description="Find AI coaching assistants"` on the Discover
 * header and both onboarding profile cards sat behind a gate reporting zero
 * because the prop name was not in the list. Two or more words, so a key table
 * (`'app.sportRunning'`) and an enum value (`'north_star'`) stay out.
 */
const CONST_PROSE = /^\s*(?:export\s+)?const\s+[A-Z_][A-Z0-9_]*\s*=\s*(?:'([A-Z][^'\n]*\s[^'\n]{2,})'|"([A-Z][^"\n]*\s[^"\n]{2,})")/gm;

/**
 * Prose handed to a function, rather than rendered.
 *
 * `setError('Login failed')` and `err.message || 'Could not connect'` are both
 * strings a user reads, and neither is a text node, a prop, an object literal
 * or a ternary — so every pattern above walked past them. The login form's
 * hardcoded English survived three i18n passes in that blind spot.
 *
 * Requires two words to stay out of identifiers and enum-ish arguments.
 *
 * A comma counts as an opener, not just `(`. Anchoring only on `(` meant the
 * pattern saw the FIRST argument of a call and nothing after it, so
 * `Alert.alert(t('common.error'), 'Failed to revoke token')` reported clean —
 * and so did every multi-line call, where the argument starts on its own line.
 * That gap is how SettingsScreen scored zero while holding three English
 * strings.
 */
const CALL_ARG_PROSE = /(?:\(|\|\||\?\?)\s*'([A-Z][a-z]+(?: [^']{2,110})?)'/g;

/**
 * Prose in a LATER argument: `Alert.alert(t('…'), 'Failed to revoke token')`.
 *
 * Requires two words, which is what keeps `const TABS = ['Overview', 'Details']`
 * out. A comma is an argument separator and an array separator both, and a
 * regex cannot tell them apart — but a one-word capture after a comma is far
 * more often an array element or an enum member than a sentence, and a real
 * sentence has a space in it. Anchoring on `(` alone was the previous bug: it
 * saw the first argument of a call and nothing after it.
 */
const CALL_LATER_ARG_PROSE = /,\s*'([A-Z][a-z]+ [^']{2,110})'/g;
const OBJECT_LITERAL =
  /\b(?:name|label|title|description|heading|text|message|placeholder)\s*:\s*'([A-Z][^']{2,120})'/g;
/**
 * Both arms of a ternary inside an expression slot: `{busy ? 'Signing in…' :
 * 'Sign in'}`. Prose hides here in every loading state and every toggle label,
 * and it is neither a text node nor a plain prop, so the two patterns above
 * walk straight past it. Requiring a capital on both arms keeps className
 * ternaries — which are lowercase utility strings — out.
 */
const TERNARY_PROSE = /\?\s*'([A-Z][^']{2,120})'\s*:\s*'([A-Z][^']{2,120})'/g;
/**
 * The same, double-quoted. TypeScript reaches for double quotes precisely when
 * the copy contains an apostrophe — "You're all caught up!" sat on the
 * notifications screen in English because only the single-quoted form was
 * being looked for.
 */
const TERNARY_PROSE_DQ = /\?\s*"([A-Z][^"]{2,120})"\s*:\s*"([A-Z][^"]{2,120})"/g;
/** A single quoted arm paired with a non-string arm: `x ? "Done!" : null`. */
const TERNARY_SINGLE_ARM = /[?:]\s*"([A-Z][^"]{4,120})"/g;
/** The same in single quotes: `n > 0 ? `${n} unread` : 'All caught up'`. */
const TERNARY_SINGLE_ARM_SQ = /[?:]\s*'([A-Z][^']{4,120})'/g;
/**
 * Prose inside a template literal. `${n} unread` is a sentence with a value in
 * it, and it is neither a text node nor a quoted string — the notifications
 * header carried one in English through every pass before this.
 */
const TEMPLATE_PROSE = /`(?:\$\{[^}]*\}|[^`$])*`/g;

/**
 * Shapes that read as prose but are not: URLs, paths, SCREAMING_CASE tokens,
 * identifiers, and anything carrying template or arrow syntax.
 */
const NOT_PROSE = /^(?:https?:|\/|#|[A-Z_]+$)|^[A-Z][a-z]+[A-Z]|\{|\}|=>|\.tsx?$/;

/**
 * Code that reads as prose once JSX text is allowed to span lines.
 *
 * Widening the text pattern to cross newlines made `=>` on one line and a
 * `<HTMLElement>` generic on the next look like a `>...<` pair, so
 * `Array.from( aside.querySelectorAll` was captured, translated, and wired —
 * which broke the file it was in. A call or index opener, an arrow, or an
 * identifier chained onto another with no space after the dot are shapes UI
 * copy does not have. Entities are resolved first: `&apos;` carries a
 * semicolon, and testing before decoding rejected seven real sentences.
 *
 * A closing bracket counts as much as an opening one: a type annotation
 * `(key: string, opts?: Record<string, unknown>) => string` puts `=>` and
 * `Record<` on the page, and the pair between them read as a text node.
 */
const CODE_SHAPE = /[()[\]]|=>|\w\.\w|\?\.|\$\{|`/;

const ENTITIES: Record<string, string> = {
  '&apos;': "'", '&quot;': '"', '&amp;': '&',
  '&lt;': '<', '&gt;': '>', '&mdash;': '\u2014', '&rarr;': '\u2192', '&nbsp;': ' ',
};

function decodeEntities(text: string): string {
  return Object.entries(ENTITIES).reduce((acc, [ent, ch]) => acc.split(ent).join(ch), text);
}

/**
 * Where an athlete ENTERS the web app. Everything reachable from these is
 * athlete scope, derived by import walk.
 *
 * This was a list of ~60 surfaces, one entry per component, and it was wrong by
 * 175 strings. `isAthleteSurface` matched a PATH, so listing `UserSettings.tsx`
 * covered exactly that file and none of the six tabs it renders — every one of
 * which an athlete opens. Five of those tabs had a mobile twin that WAS
 * translated, because the twin lives under `screens/` and was listed: the same
 * athlete read French AI settings, messaging settings and coaching style on
 * their phone and English in the browser, off one `users.locale`.
 *
 * Entry points change when navigation changes. A component list changes every
 * time anyone adds a tab, and nothing fails when they forget — which is how a
 * GDPR disclosure came to be scored as operator chrome.
 */
const WEB_ENTRY_POINTS = [
  // Sidebar destinations a non-admin gets — `regularTabs` in Dashboard.tsx.
  'components/ChatTab.tsx',
  'components/StoreScreen.tsx',
  'components/notifications/NotificationsPanel.tsx',
  'components/BillingPage.tsx',
  // Settings, and the Data Providers pane, which renders as
  // `<UserSettings initialTab="connections" hideTabNav />`. An athlete sees
  // EVERY SETTINGS_TABS entry: ADMIN_HIDDEN_TABS removes tabs from admins.
  'components/UserSettings.tsx',
  // Signup through first coach.
  'components/OnboardingFlow.tsx',
  // Unauthenticated surfaces — reachable before any role exists.
  'components/Login.tsx',
  'components/Register.tsx',
  'components/ForgotPassword.tsx',
  'components/ResetPassword.tsx',
  'components/VerifyEmail.tsx',
  'components/PendingApproval.tsx',
  'components/OAuthCallback.tsx',
  'components/ErrorBoundary.tsx',
];

/**
 * The operator console ships English, and that is now a decision rather than an
 * inference from an empty scan.
 *
 * 683 distinct strings once the athlete surface reached zero: user management,
 * the eval harness,
 * tool and harness config, claim verdicts, API-token administration. Operators
 * are internal staff, the console has never been translated, and nothing in the
 * product offers to translate it.
 *
 * What makes it a decision and not an omission is that the alternative was
 * measured. `users.locale` is `NOT NULL DEFAULT 'fr'` on `users` — not on an
 * athlete table — so every operator carries a locale, and `App.tsx` mounts the
 * language switcher with no role gate. An operator on the default locale
 * therefore reads French navigation around an English console today. That is
 * accepted deliberately: the shell follows the viewer, the console does not,
 * and translating 663 operator strings buys nothing for an internal audience.
 *
 * The figure ROSE from 668 while athlete strings were being translated, which
 * looks wrong and is not. `countsByScope` gives a string rendered on both
 * surfaces to the athlete, so a word like `Cancel` sitting in an athlete file
 * and an operator one counted once, as athlete. Translating the athlete copy
 * leaves the operator copy alone in the count. Nothing was added.
 *
 * The number is recorded rather than asserted, so it says what was decided
 * without failing a build when an operator surface legitimately grows. The
 * ratchet prints the live figure on every run; a reader comparing the two sees
 * drift immediately.
 *
 * What IS asserted is the other direction. A derived scope can shrink silently
 * — miss a barrel or a lazy import and those files leave athlete scope, the
 * ceiling of 0 keeps passing, and the gate reports success while measuring
 * less. That is how this ratchet failed three times in one day. The canaries in
 * `untranslatedScanShapes.test.ts` pin both directions so it fails loudly
 * instead.
 */

/** Resolve one relative import to a real file, following barrels. */
/** The monorepo root above a `frontend/`, `frontend-mobile/` or `packages/` file. */
function repoRootOf(file: string): string | null {
  const parts = file.split(path.sep);
  const at = parts.findIndex((p) => p === 'frontend' || p === 'frontend-mobile' || p === 'packages');
  return at <= 0 ? null : parts.slice(0, at).join(path.sep);
}

/**
 * Where an import specifier leads.
 *
 * Relative specifiers resolve beside the importer. A `@pierre/<package>` one
 * resolves into that workspace package's `src/` — the label tables in
 * `@pierre/shared-constants` and the verdict vocabulary in `@pierre/shared-types`
 * are read by the athlete's screens exactly as a component is, so the walk
 * follows them; a package no athlete entry point reaches stays operator scope
 * by the same rule that governs a component.
 */
function resolveImport(fromFile: string, spec: string): string | null {
  // The shared packages import ESM-style, `./verdict.js` naming the `.ts`
  // beside it; the extension names the emitted file, so it is dropped before
  // the candidates below are tried.
  const stem = spec.replace(/\.(?:js|jsx|mjs)$/, '');
  let base: string;
  if (stem.startsWith('.')) {
    base = path.resolve(path.dirname(fromFile), stem);
  } else if (stem.startsWith('@pierre/')) {
    const root = repoRootOf(fromFile);
    if (root === null) {
      return null;
    }
    const [pkg, ...rest] = stem.slice('@pierre/'.length).split('/');
    base = path.join(root, 'packages', pkg, 'src', ...(rest.length === 0 ? ['index'] : rest));
  } else {
    return null;
  }
  const candidates = [
    `${base}.tsx`,
    `${base}.ts`,
    // A barrel: `components/ui` and `components/dashboard` are reached this way,
    // and a walk that stops at the directory loses everything behind them.
    path.join(base, 'index.tsx'),
    path.join(base, 'index.ts'),
  ];
  return candidates.find((c) => fs.existsSync(c) && fs.statSync(c).isFile()) ?? null;
}

/**
 * Every import specifier, in all three spellings that reach a component.
 *
 * The lazy form is not optional: `Dashboard.tsx` mounts most panes through
 * `lazy(() => import('./X'))`, and a walk that sees only `from '…'` misses
 * them. A first draft of this missed exactly that and misclassified
 * `UnifiedConnections`.
 */
// The window between `export {` and `from` is wide: `@pierre/shared-types`'s
// barrel re-exports twenty names per line group, and a 400-character window
// stopped short of the specifier, which left `verdict.ts` unreachable and its
// chip vocabulary scored as operator chrome. Lazy, so it still stops at the
// nearest `from`.
const IMPORT_SPEC =
  /(?:(?:import|export)[\s\S]{0,4000}?from\s*|import\s*\(\s*|^\s*import\s+)['"]([^'"]+)['"]/gm;

/** Memoised per web root — the walk reads ~125 files and the scan calls this per hit. */
const reachableCache = new Map<string, Set<string>>();

/** Every module an athlete can reach from `webRoot`, transitively. */
function athleteReachable(webRoot: string): Set<string> {
  const cached = reachableCache.get(webRoot);
  if (cached) {
    return cached;
  }
  const seen = new Set<string>();
  const queue: string[] = [];
  for (const entry of WEB_ENTRY_POINTS) {
    const full = path.join(webRoot, entry);
    if (fs.existsSync(full)) {
      seen.add(full);
      queue.push(full);
    }
  }
  while (queue.length > 0) {
    const file = queue.shift();
    if (file === undefined) {
      break;
    }
    let source: string;
    try {
      source = fs.readFileSync(file, 'utf-8');
    } catch {
      continue;
    }
    IMPORT_SPEC.lastIndex = 0;
    let match = IMPORT_SPEC.exec(source);
    while (match !== null) {
      const resolved = resolveImport(file, match[1]);
      if (resolved !== null && !seen.has(resolved)) {
        seen.add(resolved);
        queue.push(resolved);
      }
      match = IMPORT_SPEC.exec(source);
    }
  }
  reachableCache.set(webRoot, seen);
  return seen;
}

/** The `frontend/src` directory `file` lives under, or null if it does not. */
function webRootOf(file: string): string | null {
  const marker = `${path.sep}frontend${path.sep}src${path.sep}`;
  const at = file.indexOf(marker);
  return at === -1 ? null : file.slice(0, at + marker.length - 1);
}

/**
 * Whether a component renders on a surface an athlete can reach.
 *
 * Mobile is unconditional: the app has no operator console, so every file in it
 * is athlete-facing. Fifteen `screens/*` fragments used to stand in for that
 * rule, and anything outside `screens/` fell through — which is how
 * `Server unreachable`, `Checking...` and `Something went wrong. Please try
 * again.` shipped in English to an app whose default locale is French.
 */
export function isAthleteSurface(file: string): boolean {
  const normalized = file.split(path.sep).join('/');
  if (normalized.includes('/frontend-mobile/')) {
    return true;
  }
  if (normalized.includes('/packages/')) {
    // A shared package is athlete surface when an athlete entry point of the
    // web app imports it, directly or through a barrel — the walk decides,
    // not the package's name.
    const root = repoRootOf(file);
    return root === null ? false : athleteReachable(path.join(root, 'frontend', 'src')).has(file);
  }
  const webRoot = webRootOf(file);
  return webRoot === null ? false : athleteReachable(webRoot).has(file);
}

/** A hardcoded string and where it renders. */
export interface UntranslatedString {
  file: string;
  text: string;
  /** `athlete` strings are in scope for translation; `operator` ship English. */
  scope: 'athlete' | 'operator';
}

/**
 * Comments are not copy. Since the text pattern may close on `{` as well as
 * `<`, a prose comment sitting between two tags reads as a text node — one
 * explaining a routing decision was picked up and very nearly translated.
 */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/^\s*\/\/.*$/gm, ' ');
}

/**
 * Console output is read by developers, not athletes.
 *
 * The call-argument pattern cannot tell `setError('Could not connect')` from
 * `console.error('Failed to open URL:', err)`, and the second is not copy. The
 * calls are blanked before scanning rather than filtered afterwards, so a
 * sentence that appears BOTH in a console call and in real UI is still caught
 * where it matters.
 */
function stripConsole(source: string): string {
  return source.replace(/console\.\w+\([^)]*\)/g, ' ');
}

/**
 * A font family is a typeface's name, not copy.
 *
 * `fontFamily: 'Menlo'` and `fontFamily: 'Inter_Medium'` read as capitalised
 * words to every pattern above, and translating either would silently break
 * the type rather than the sentence. Blanked for the same reason console calls
 * are: the alternative is an exception list naming individual typefaces, which
 * would need editing every time a face is added.
 */
function stripFontFamily(source: string): string {
  return source.replace(/fontFamily:\s*(?:'[^']*'|"[^"]*"|[^,\n}]+)/g, ' ');
}

/**
 * A string being MATCHED is not a string being shown.
 *
 * ErrorBoundary tests `error.message.includes('Loading chunk')` to detect a
 * stale bundle after a deploy and reload the page. That literal is the
 * browser's own wording, and translating it would not change a sentence — it
 * would silently break the auto-reload. Nothing user-facing is ever passed to
 * `.includes()`, `.startsWith()` or `.endsWith()`, so the arguments are blanked.
 */
/**
 * Props whose value is never copy: styling and test hooks.
 *
 * Widening the template-literal filter to admit sentence punctuation also
 * admitted `className={`text-xs font-medium`}` and
 * `testID={`notification-pref-cap-${id}`}` — hyphenated lowercase tokens read
 * as a sentence once hyphens are allowed. Blanking the props is precise and
 * needs no guessing about shape; a className is never shown to anyone.
 */
function stripNonCopyProps(source: string): string {
  return source.replace(
    /\b(?:className|testID|style|key|id|htmlFor|data-testid)=\{`(?:[^`]|\$\{[^}]*\})*`\}/g,
    ' ',
  )
    .replace(/\b(?:className|testID|style|key|id|htmlFor|data-testid)="[^"]*"/g, ' ')
    // A filename or URL being ASSIGNED is not copy either:
    // `a.download = \`billing-${period}.${format}\`` is what a saved export is
    // called on disk, and translating it would rename the file.
    .replace(/\.(?:download|href|src|action|pathname)\s*=\s*`(?:[^`]|\$\{[^}]*\})*`/g, ' ');
}

/**
 * React Native's dev log-suppression list.
 *
 * `LogBox.ignoreLogs(['Failed to send message:', …])` is a set of PREFIXES
 * matched against warnings in development. They are shaped exactly like error
 * copy because they are quoting error copy — but translating them would only
 * stop the suppression matching.
 */
function stripLogBox(source: string): string {
  return source.replace(/LogBox\.\w+\(\[[^\]]*\]\)/g, ' ');
}

function stripMatchers(source: string): string {
  return source.replace(/\.(?:includes|startsWith|endsWith)\([^)]*\)/g, ' ');
}

function collect(input: string): string[] {
  const source = stripLogBox(
    stripNonCopyProps(
      stripMatchers(stripFontFamily(stripConsole(stripComments(input)))),
    ),
  );
  const found = new Set<string>();
  /**
   * Captures whose SYNTAX already proves they are copy — the key names them
   * (`text:`, `helpText=`). CODE_SHAPE is not applied to these.
   *
   * `{ text: 'Member (athlete)' }` is a dialog button label that stayed English
   * because CODE_SHAPE rejects anything containing a parenthesis. That filter
   * exists to throw out loose JSX-text captures that are really code; a value
   * sitting behind `text:` is not a guess, and real copy contains brackets.
   */
  const trusted = new Set<string>();
  for (const re of [STRING_PROP, OBJECT_LITERAL]) {
    re.lastIndex = 0;
    let m = re.exec(source);
    while (m !== null) {
      trusted.add(m[1].replace(/\s+/g, ' ').trim());
      m = re.exec(source);
    }
  }
  for (const re of [JSX_TEXT, STRING_PROP, OBJECT_LITERAL, CALL_ARG_PROSE, CALL_LATER_ARG_PROSE, CONST_PROSE]) {
    re.lastIndex = 0;
    let m = re.exec(source);
    while (m !== null) {
      // JSX text may wrap across lines; the reader sees one collapsed string.
      const text = m[1].replace(/\s+/g, ' ').trim();
      if (/^[A-Z]/.test(text) || text.includes(' ')) {
        found.add(text);
      }
      m = re.exec(source);
    }
  }
  for (const re of [TERNARY_PROSE, TERNARY_PROSE_DQ]) {
    re.lastIndex = 0;
    let t = re.exec(source);
    while (t !== null) {
      found.add(t[1].trim());
      found.add(t[2].trim());
      t = re.exec(source);
    }
  }
  for (const re of [TERNARY_SINGLE_ARM, TERNARY_SINGLE_ARM_SQ]) {
    re.lastIndex = 0;
    let a = re.exec(source);
    while (a !== null) {
      found.add(a[1].trim());
      a = re.exec(source);
    }
  }
  TEMPLATE_PROSE.lastIndex = 0;
  let g = TEMPLATE_PROSE.exec(source);
  while (g !== null) {
    // The words around the holes. Two or more of them is a sentence, not a
    // class list or a URL fragment.
    const words = g[0].slice(1, -1).replace(/\$\{[^}]*\}/g, ' ').replace(/\s+/g, ' ').trim();
    // Sentence punctuation is allowed. The previous charset was
    // `[A-Za-z' ]`, which rejected every capture containing `?` or `"` — i.e.
    // every confirmation prompt in the app (`Leave "${name}"?`). Ten Alert
    // bodies stayed English behind that one character class while their titles
    // translated around them.
    if (/^[A-Za-z][A-Za-z'"?!.,:; —-]{4,}$/.test(words) && words.includes(' ')) {
      found.add(words);
    }
    g = TEMPLATE_PROSE.exec(source);
  }
  return [...found].filter(
    (text) =>
      text.length >= 3 &&
      /[a-z]/.test(text) &&
      !NOT_PROSE.test(text) &&
      // CODE_SHAPE only applies to the LOOSE captures. A value behind a named
      // label key is copy by construction, and real copy contains brackets.
      (trusted.has(text) || !CODE_SHAPE.test(decodeEntities(text))),
  );
}

function walk(dir: string, out: string[]): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== '__tests__') {
        walk(full, out);
      }
    } else if (entry.name.endsWith('.tsx')) {
      out.push(full);
    }
  }
  return out;
}

/** Every user-facing string still hardcoded under `roots`. */
export function scanUntranslated(roots: string[]): UntranslatedString[] {
  const hits: UntranslatedString[] = [];
  for (const root of roots) {
    if (!fs.existsSync(root)) {
      continue;
    }
    // A root may be a single file (App.tsx) rather than a directory.
    const files = fs.statSync(root).isDirectory() ? walk(root, []) : [root];
    for (const file of files) {
      const scope = isAthleteSurface(file) ? 'athlete' : 'operator';
      for (const text of collect(fs.readFileSync(file, 'utf-8'))) {
        hits.push({ file, text, scope });
      }
    }
  }
  return hits;
}

/** Distinct strings, which is what the gate counts — the same word twice is one translation. */
export function distinctCount(hits: UntranslatedString[]): number {
  return new Set(hits.map((h) => h.text)).size;
}

/**
 * Distinct strings per scope.
 *
 * A string rendered on both an athlete and an operator surface counts as
 * athlete: an athlete reads it, so it needs translating regardless of where
 * else it appears.
 */
export function countsByScope(hits: UntranslatedString[]): { athlete: number; operator: number } {
  const athlete = new Set(hits.filter((h) => h.scope === 'athlete').map((h) => h.text));
  const operator = new Set(
    hits.filter((h) => h.scope === 'operator').map((h) => h.text).filter((t) => !athlete.has(t)),
  );
  return { athlete: athlete.size, operator: operator.size };
}
