// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The feedback washes that are not alerts — a faint hue behind content that keeps its own body or bound ink
// ABOUTME: The version history's before/after cells, the private lane's composer, the current plan card; each fill beside the inks drawn on it

/**
 * A wash marks where something sits — what changed, which lane a message goes
 * to, which plan is yours — under content that stays ordinary text. An alert
 * speaks in its tone's ink; a wash leaves the words in body ink (or, for the
 * private lane's quote label, the hue's bound ink), so it never pairs a bare
 * hue with its own tint. Every component that draws one takes its fill and
 * inks from here, and alertTone.test.tsx measures each ink on its fill over
 * every ground in both schemes.
 */
export const WASH_TONE = {
  /** A version's old value. */
  versionBefore: { fill: 'bg-error/10', inks: { value: 'text-on-surface', label: 'text-on-surface-variant' } },
  /** A version's new value. */
  versionAfter: { fill: 'bg-success/10', inks: { value: 'text-on-surface', label: 'text-on-surface-variant' } },
  /** The composer while a message goes to the private lane, and its quote chip. */
  privateLane: {
    fill: 'bg-warning/5',
    inks: { typed: 'text-on-surface', quoteLabel: 'text-on-warning-container', quoted: 'text-on-surface-variant' },
  },
  /** The plan card of the plan you are on. */
  currentPlan: { fill: 'bg-success/5', inks: { value: 'text-on-surface', label: 'text-on-surface-variant' } },
} as const;
