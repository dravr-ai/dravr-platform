// @vitest-environment node
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Lints fixtures through the real web and mobile ESLint configs
// ABOUTME: Proves each refuses a thrown error's raw message and accepts the shared classifier

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { ESLint } from 'eslint';
import { describe, expect, it } from 'vitest';

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../..');

/**
 * Each config that lints code an athlete can see, with a path inside it that
 * the restriction must cover. The mobile config imports the shared entries by
 * a relative path and the web config through the package, so both are checked
 * rather than one standing in for the other.
 */
const CONFIGS = [
  { name: 'web', cwd: path.join(REPO_ROOT, 'frontend'), file: 'src/components/Fixture.tsx' },
  { name: 'mobile', cwd: path.join(REPO_ROOT, 'frontend-mobile'), file: 'src/screens/Fixture.tsx' },
] as const;

const PREAMBLE = `
declare const t: (key: string) => string;
declare function describeApiError(err: unknown, opts: { t: (key: string) => string; fallbackKey: string }): string;
declare function show(text: string): void;
`;

/** The three ways a screen read the thrown error's own text as its sentence. */
const REFUSED = {
  ternary: `${PREAMBLE}
export function onFailure(err: unknown): void {
  show(err instanceof Error ? err.message : t('app.failedLoadTokens'));
}
`,
  guardedTernary: `${PREAMBLE}
export function onFailure(err: unknown): void {
  show(err instanceof Error && err.message ? err.message : t('app.failedLoadTokens'));
}
`,
  assignedInIf: `${PREAMBLE}
export function onFailure(err: unknown): void {
  let message = t('app.somethingWentWrongRetry');
  if (err instanceof Error) {
    message = err.message;
  }
  show(message);
}
`,
} as const;

/** What the sites were moved to, and a narrowing that classifies without rendering. */
const ACCEPTED = {
  classifier: `${PREAMBLE}
export function onFailure(err: unknown): void {
  show(describeApiError(err, { t, fallbackKey: 'app.failedLoadTokens' }));
}
`,
  narrowedCode: `${PREAMBLE}
export function failureKey(err: unknown): string {
  if (!(err instanceof Error)) {
    return 'auth.googleSignInFailed';
  }
  switch (err.message) {
    case 'GOOGLE_SIGNIN_UNAVAILABLE':
      return 'errors.googleSignInUnavailable';
    default:
      return 'auth.googleSignInFailed';
  }
}
`,
} as const;

async function restrictionHits(cwd: string, file: string, source: string): Promise<string[]> {
  const eslint = new ESLint({ cwd });
  const [result] = await eslint.lintText(source, { filePath: path.join(cwd, file) });
  return result.messages
    .filter((message) => message.ruleId === 'no-restricted-syntax')
    .map((message) => message.message);
}

describe.each(CONFIGS)('the $name ESLint config', ({ cwd, file }) => {
  it.each(Object.entries(REFUSED))('refuses the %s shape, naming the classifier', async (_shape, source) => {
    const hits = await restrictionHits(cwd, file, source);
    expect(hits).toHaveLength(1);
    expect(hits[0]).toContain('describeApiError(err, { t, fallbackKey })');
  });

  it.each(Object.entries(ACCEPTED))('accepts the %s shape', async (_shape, source) => {
    expect(await restrictionHits(cwd, file, source)).toEqual([]);
  });
});
