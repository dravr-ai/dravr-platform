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
const JSX_TEXT = />\s*([A-Za-z][^<>{}]{2,200}?)\s*[<{]/g;
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
  /\b(?:placeholder|aria-label|accessibilityLabel|accessibilityHint|title|alt|label)="([A-Z][^"]{2,120})"/g;

/**
 * Prose handed to a function, rather than rendered.
 *
 * `setError('Login failed')` and `err.message || 'Could not connect'` are both
 * strings a user reads, and neither is a text node, a prop, an object literal
 * or a ternary — so every pattern above walked past them. The login form's
 * hardcoded English survived three i18n passes in that blind spot.
 *
 * Requires two words to stay out of identifiers and enum-ish arguments.
 */
const CALL_ARG_PROSE = /(?:\(|\|\|\s|\?\?\s)'([A-Z][a-z]+(?: [^']{2,110})?)'/g;
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
 * Surfaces an athlete can reach, and therefore the ones that must speak their
 * language. Everything else in `components/` is operator chrome — user
 * management, the eval harness, tool and harness config, claim verdicts — and
 * ships in English deliberately, not by omission: operators are internal, and
 * nobody needs Harness Config in Portuguese.
 *
 * A list of surfaces rather than a list of excused strings. It says which parts
 * of the product are athlete-facing, the same contract the shared surface
 * registry keeps for routes; adding an athlete surface means adding it here, and
 * the ceiling below then covers it.
 */
const ATHLETE_SURFACES = [
  // ── frontend-mobile ──────────────────────────────────────────────────────
  // The mobile app is athlete-facing almost end to end: it has no operator
  // console. Its paths are `screens/<area>`, which none of the web entries
  // below match, so before these were listed the whole app scored as operator
  // chrome and the ceiling never saw it.
  'screens/chat', 'screens/conversations', 'screens/groups', 'screens/coaches',
  'screens/connections', 'screens/settings', 'screens/onboarding',
  'screens/notifications', 'screens/store', 'screens/memory', 'screens/auth',
  'SciotteLoginModal.tsx', 'IntervalsIcuLinkModal.tsx',
  'OAuthCredentialsSection.tsx', 'OAuthAppSetupModal.tsx',
  'CommandPalette.tsx', 'MentionPalette.tsx', 'VoiceButton.tsx',

  // ── frontend (web) ───────────────────────────────────────────────────────
  'components/chat', 'components/groups', 'components/discover',
  'components/notifications', 'components/dashboard', 'components/ui',
  'components/memory', 'components/layout', 'onboarding',
  'ChatTab.tsx', 'StoreScreen.tsx', 'UserSettings.tsx', 'NotificationSettingsTab.tsx',
  'PromptSuggestions.tsx', 'Dashboard.tsx', 'CommandPalette.tsx',
  'Login.tsx', 'Register.tsx', 'ForgotPassword.tsx', 'ResetPassword.tsx',
  'VerifyEmail.tsx', 'PendingApproval.tsx', 'OnboardingFlow.tsx',
  'OnboardingAboutYou.tsx', 'OnboardingParq.tsx', 'OnboardingCoachProposal.tsx',
  'OnboardingConnectProvider.tsx', 'OnboardingMessagingChannel.tsx',
  'OnboardingMessagingConfigure.tsx', 'SciotteLoginModal.tsx',
  'IntervalsIcuLinkModal.tsx', 'ConnectPreview.tsx', 'ConnectProviderBanner.tsx',
  'BillingPage.tsx', 'BillingTab.tsx', 'ErrorBoundary.tsx', 'OAuthCallback.tsx',
  'ImpersonationBanner.tsx', 'LanguageSwitcher.tsx',
];

/** Whether a component renders on a surface an athlete can reach. */
export function isAthleteSurface(file: string): boolean {
  const normalized = file.split(path.sep).join('/');
  return ATHLETE_SURFACES.some(
    (surface) => normalized.includes(`/${surface}/`) || normalized.endsWith(`/${surface}`),
  );
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
function stripMatchers(source: string): string {
  return source.replace(/\.(?:includes|startsWith|endsWith)\([^)]*\)/g, ' ');
}

function collect(input: string): string[] {
  const source = stripMatchers(stripFontFamily(stripConsole(stripComments(input))));
  const found = new Set<string>();
  for (const re of [JSX_TEXT, STRING_PROP, OBJECT_LITERAL, CALL_ARG_PROSE]) {
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
    const words = g[0].slice(1, -1).replace(/\$\{[^}]*\}/g, ' ').trim();
    if (/^[A-Za-z][A-Za-z' ]{4,}$/.test(words) && words.includes(' ')) {
      found.add(words);
    }
    g = TEMPLATE_PROSE.exec(source);
  }
  return [...found].filter(
    (text) =>
      text.length >= 3 &&
      /[a-z]/.test(text) &&
      !NOT_PROSE.test(text) &&
      !CODE_SHAPE.test(decodeEntities(text)),
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
