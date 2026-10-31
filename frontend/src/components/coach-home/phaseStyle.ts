// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: How a plan phase is painted on Coach Home — a tint per kind, a dashed outline for the quiet ones and for a draft
// ABOUTME: Tokens only; the same bands draw the athlete pane's plan, the season view's rows and the phone's countdown

import type { CoachRosterPhaseKind } from '@pierre/shared-types';

const PHASE_CLASS: Record<CoachRosterPhaseKind | 'draft', string> = {
  prep: 'bg-surface-container-high text-on-surface-variant',
  base: 'bg-surface-container-high text-on-surface-variant',
  build: 'bg-info/15 text-on-info-container',
  specialty: 'bg-primary/20 text-on-primary-container',
  peak: 'bg-warning/20 text-on-warning-container',
  taper: 'bg-warning/20 text-on-warning-container',
  race: 'bg-on-surface text-surface',
  transition: 'border border-dashed border-outline text-on-surface-variant',
  recovery: 'border border-dashed border-outline text-on-surface-variant',
  draft: 'border-[1.5px] border-dashed border-primary text-primary',
};

/** The classes a phase band is painted with. */
export function phaseClass(kind: CoachRosterPhaseKind | 'draft'): string {
  return PHASE_CLASS[kind];
}
