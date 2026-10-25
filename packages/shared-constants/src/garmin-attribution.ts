// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The line Garmin's brand guidelines owe a figure derived from Garmin-sourced data, in the reader's language
// ABOUTME: The server says what Garmin contributed; both clients print this line beneath the figure's title, never in a tooltip

import type { GarminContribution } from '@pierre/shared-types';

/**
 * The corpus key for each contribution (carnet#1154). Garmin's guidelines ask
 * a derived figure to name Garmin as its distinct or contributing source,
 * whichever is true, without implying Garmin stands behind the other sources'
 * data; the words follow the acceptable samples they give.
 */
export const GARMIN_CONTRIBUTION_KEY: Record<GarminContribution, string> = {
  sole: 'garminAttribution.sole',
  part: 'garminAttribution.part',
};

/** The shape of `t()` this module needs. */
type Translate = (key: string) => string;

/**
 * The line a figure carries directly beneath its title, or `undefined` when
 * no Garmin-sourced data went into it.
 */
export function garminContributionLine(
  t: Translate,
  contribution: GarminContribution | null | undefined,
): string | undefined {
  return contribution ? t(GARMIN_CONTRIBUTION_KEY[contribution]) : undefined;
}
