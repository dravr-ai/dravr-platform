// @vitest-environment node
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Lints fixtures through the real web ESLint config to prove the design rule fires
// ABOUTME: A rule about how a class may be spelled anywhere in the tree is a lint rule, and one nobody has seen fire guards nothing

import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { restrictionHits } from '../../../packages/eslint-config-pierre/testing/lintFixture.cjs';

// The mobile rules are proved in frontend-mobile/__tests__/designRuleLint.test.ts,
// in the suite a mobile-only push runs; this file proves web's.
const WEB_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');

/**
 * The web config lists the rule in two blocks, because a file's
 * no-restricted-syntax list is whichever block matched last: one for every
 * file under src/, one for the components outside ui/ that adds the
 * form-control rules. A fixture under each proves each list carries it.
 */
const TARGETS = [
  ['a component', { cwd: WEB_ROOT, file: 'src/components/Fixture.tsx' }],
  ['a ui primitive', { cwd: WEB_ROOT, file: 'src/components/ui/Fixture.tsx' }],
] as const;

// Spelled in two halves so this file does not carry the class it tests for:
// the web config lints test files too.
const INVERT = ['prose', 'invert'].join('-');

describe.each(TARGETS)('web: the dark typography palette is dark-only, in %s', (_where, WEB) => {
  it.each([
    ['a class string', `export const C = () => <div className="prose ${INVERT} max-w-none" />;`],
    ['a template class', `export const C = (p: { e: boolean }) => <div className={\`prose ${INVERT} \${p.e ? 'x' : ''}\`} />;`],
    ['a helper argument', `declare function clsx(...a: string[]): string;\nexport const c = clsx('prose', '${INVERT}');`],
  ])('refuses the bare class in %s', async (_shape, source) => {
    const found = await restrictionHits(WEB, source);
    expect(found).toHaveLength(1);
    expect(found[0].ruleId).toBe('no-restricted-syntax');
    expect(found[0].message).toContain(`Write dark:${INVERT}`);
  });

  it('accepts the dark variant, alone and behind another variant', async () => {
    expect(await restrictionHits(WEB, `export const C = () => <div className="prose dark:${INVERT} max-w-none" />;`)).toEqual([]);
    expect(await restrictionHits(WEB, `export const C = () => <div className={\`prose md:dark:${INVERT}\`} />;`)).toEqual([]);
  });
});
