// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Types for lintFixture.cjs, the fixture linter the web, mobile and SDK suites share
// ABOUTME: Lets the TypeScript suites import the CommonJS helper under strict mode

/** A project and the path, relative to it, a fixture is linted as. */
export interface LintTarget {
  /** The project directory whose ESLint config and ESLint install are used. */
  cwd: string;
  /** Where the fixture pretends to live; decides which config blocks apply. */
  file: string;
}

/** The restriction rules a hit can come from. */
export type RestrictionRule = 'no-restricted-syntax' | 'no-restricted-imports';

/** One report from a restriction rule: which rule fired, and what it said. */
export interface RestrictionHit {
  ruleId: RestrictionRule;
  message: string;
}

/** What the project's no-restricted-syntax / no-restricted-imports rules report for `source`. */
export function restrictionHits(target: LintTarget, source: string): Promise<RestrictionHit[]>;
