// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Makes the scanner fail on purpose, one syntactic shape at a time
// ABOUTME: A ceiling of 0 means nothing unless the scanner can SEE each shape it claims to

import fs from 'fs';
import os from 'os';
import path from 'path';
import { describe, it, expect, afterEach } from 'vitest';
import { scanUntranslated } from '../untranslatedScan';

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
