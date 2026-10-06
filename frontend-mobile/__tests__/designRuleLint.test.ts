// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Lints fixtures through the real mobile ESLint config to prove each design rule fires
// ABOUTME: A rule about what may be written anywhere in the tree is a lint rule, and one nobody has seen fire guards nothing

import path from 'path';
import {
  restrictionHits,
  type RestrictionHit,
  type RestrictionRule,
} from '../../packages/eslint-config-pierre/testing/lintFixture.cjs';

/**
 * This lives in the mobile suite because that is what a mobile-only push runs:
 * "CI: Mobile Unit" lints and runs jest here, never the web suite. Proved from
 * the web side, a rule dropped from eslint.config.js went green.
 */
const MOBILE_ROOT = path.resolve(__dirname, '..');
const SCREEN = { cwd: MOBILE_ROOT, file: 'src/screens/Fixture.tsx' };
const ROUTE = { cwd: MOBILE_ROOT, file: 'app/(app)/fixture.tsx' };

// Each fixture is linted by ESLint in its own process, on a shared runner.
jest.setTimeout(60_000);

const IMPORTS: RestrictionRule = 'no-restricted-imports';
const SYNTAX: RestrictionRule = 'no-restricted-syntax';

/**
 * The rules that fired, each once, in a fixed order. A fixture names every rule
 * it must draw: the drawn-bar restriction is written as both rules with one
 * message, so a count or a message alone stays green with either deleted.
 */
const rulesFired = (found: RestrictionHit[]): RestrictionRule[] =>
  [...new Set(found.map((hit) => hit.ruleId))].sort();

describe.each([
  ['a screen', SCREEN],
  ['a route file', ROUTE],
])("the tab bar is the platform's, in %s", (_where, target) => {
  it.each([
    // Each module is its own entry in the import rule: `paths` for the two
    // packages, a pattern for the pill under any relative path.
    ['expo-blur', `import { BlurView } from 'expo-blur';\nexport const Bar = () => <BlurView />;`],
    ['expo-glass-effect', `import { GlassContainer } from 'expo-glass-effect';\nexport const Bar = () => <GlassContainer />;`],
    ['the retired pill', `import { ExpandableTabBar } from '../components/ui/ExpandableTabBar';\nexport const Bar = () => <ExpandableTabBar />;`],
  ])('refuses %s, at the import and at the use', async (_shape, source) => {
    const found = await restrictionHits(target, source);
    // The import rule refuses the module; the syntax rule the imported name and the element.
    expect(rulesFired(found)).toEqual([IMPORTS, SYNTAX]);
    expect(found.filter((hit) => hit.ruleId === SYNTAX)).toHaveLength(3);
    for (const { message } of found) expect(message).toContain("The tab bar is the platform's");
  });

  it('refuses the component reached under a module the rule does not name', async () => {
    const found = await restrictionHits(target, `import { BlurView } from 'some-other-blur';\nexport const Bar = () => <BlurView />;`);
    expect(rulesFired(found)).toEqual([SYNTAX]);
    for (const { message } of found) expect(message).toContain("The tab bar is the platform's");
  });

  it('accepts the system tabs', async () => {
    const source = `import { NativeTabs } from 'expo-router/unstable-native-tabs';\nexport const Bar = () => <NativeTabs />;`;
    expect(await restrictionHits(target, source)).toEqual([]);
  });
});

describe('no colour the appearance setting cannot move', () => {
  it.each([
    ['a style literal', `export const s = { backgroundColor: '#00241a' };`],
    ['an upper-case literal', `export const s = { backgroundColor: '#00241A' };`],
    ['a gradient stop', `export const stops = ['#00241a', '#0d3b2e'];`],
    ['a class string', `declare const View: (p: { className: string }) => null;\nexport const C = () => <View className="bg-[#0d3b2e]" />;`],
    ['a template string', 'export const c = (a: number) => `rgba(${a}) #00241a`;'],
  ])('refuses the retired v1 primary in %s', async (_shape, source) => {
    const found = await restrictionHits(SCREEN, source);
    expect(rulesFired(found)).toEqual([SYNTAX]);
    for (const { message } of found) expect(message).toContain('The Boreal v1 primary is retired');
  });

  it('refuses a gradient drawn from a module-level palette', async () => {
    const source = `
declare const gradients: { violetCyan: readonly [string, string] };
declare const LinearGradient: (p: { colors: readonly string[]; style?: object }) => null;
export const Strip = () => <LinearGradient colors={gradients.violetCyan} style={{ height: 3, width: '100%' }} />;
`;
    const found = await restrictionHits(SCREEN, source);
    expect(found).toHaveLength(1);
    expect(found[0].ruleId).toBe(SYNTAX);
    expect(found[0].message).toContain('No gradient from a module-level palette');
  });

  it('accepts a provider brand gradient and the live palette', async () => {
    // DESIGN.md §2: third-party brand colours are fixed in both schemes.
    const source = `
declare const brand: { gradient: readonly [string, string] };
declare const useThemeColors: () => { primary: string };
declare const LinearGradient: (p: { colors: readonly string[] }) => null;
export const Strip = () => <LinearGradient colors={brand.gradient} />;
export const Fill = () => { const colors = useThemeColors(); return <LinearGradient colors={[colors.primary, '#fc4c02']} />; };
`;
    expect(await restrictionHits(SCREEN, source)).toEqual([]);
  });
});

describe('no deprecated core SafeAreaView (carnet#356)', () => {
  it("refuses React Native's own SafeAreaView", async () => {
    const source = `import { SafeAreaView, View } from 'react-native';\nexport const S = () => <SafeAreaView><View /></SafeAreaView>;`;
    const found = await restrictionHits(SCREEN, source);
    expect(rulesFired(found)).toEqual([IMPORTS]);
    expect(found).toHaveLength(1);
    expect(found[0].message).toContain("React Native's SafeAreaView is deprecated");
  });

  it('accepts the safe-area-context one, and the rest of react-native', async () => {
    const source = `import { View } from 'react-native';\nimport { SafeAreaView } from 'react-native-safe-area-context';\nexport const S = () => <SafeAreaView><View /></SafeAreaView>;`;
    expect(await restrictionHits(SCREEN, source)).toEqual([]);
  });
});
