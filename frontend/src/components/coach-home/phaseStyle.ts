// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: How a plan phase is painted on Coach Home — a tint per kind, a dashed outline for the quiet ones and for a draft
// ABOUTME: Tokens only; the same bands draw the athlete pane's plan, the season view's rows and the phone's countdown

import type { CoachRosterPhaseKind } from '@pierre/shared-types';
import { BADGE_TONE } from '../ui/badgeTone';

/**
 * Every band carries its label, so each fill is measured with its ink on every
 * ground in both schemes (phaseStyle.test.ts). Build, peak and taper are the
 * badge's info and warning tones; peak and taper sit at the badge's /15
 * because at warning/20 their ink reads 4.41:1 on the top tier in light.
 */
const PHASE_CLASS: Record<CoachRosterPhaseKind | 'draft', string> = {
  prep: 'bg-surface-container-high text-on-surface-variant',
  base: 'bg-surface-container-high text-on-surface-variant',
  build: BADGE_TONE.info.tone,
  specialty: 'bg-primary/20 text-on-primary-container',
  peak: BADGE_TONE.warning.tone,
  taper: BADGE_TONE.warning.tone,
  race: 'bg-on-surface text-surface',
  transition: 'border border-dashed border-outline text-on-surface-variant',
  recovery: 'border border-dashed border-outline text-on-surface-variant',
  draft: 'border-[1.5px] border-dashed border-primary text-primary',
};

/** The classes a phase band is painted with. */
export function phaseClass(kind: CoachRosterPhaseKind | 'draft'): string {
  return PHASE_CLASS[kind];
}
