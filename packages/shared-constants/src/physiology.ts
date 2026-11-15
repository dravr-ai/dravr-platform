// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Names Settings › Physiology in the reader's language — each anchor's label, each sport family's heading, each confidence
// ABOUTME: The server writes an anchor's value and source; both clients render the rest through these keys and their own t()

import type { AnchorConfidence, SportAnchorName, SportFamily } from '@pierre/shared-types';

/** The corpus key for each line of the physiology pane that is not an anchor's own words. */
export const PHYSIOLOGY_KEY = {
  title: 'physiology.title',
  blurb: 'physiology.blurb',
  empty: 'physiology.empty',
  loadFailed: 'physiology.loadFailed',
  asOf: 'physiology.asOf',
  noDate: 'physiology.noDate',
  range: 'physiology.range',
} as const;

/**
 * Each anchor's label, in sentence case, as a row names it. The envelope
 * notice's labels (`messaging.envelope.anchor_label.*`) are the mid-sentence
 * forms ("critical power"); a row's title opens a line, so it has its own.
 */
export const PHYSIOLOGY_ANCHOR_LABEL_KEY: Record<SportAnchorName, string> = {
  ftp: 'physiology.anchor.ftp',
  critical_power: 'physiology.anchor.critical_power',
  map: 'physiology.anchor.map',
  critical_speed: 'physiology.anchor.critical_speed',
  vvo2max: 'physiology.anchor.vvo2max',
  threshold_speed: 'physiology.anchor.threshold_speed',
  max_heart_rate: 'physiology.anchor.max_heart_rate',
  threshold_heart_rate: 'physiology.anchor.threshold_heart_rate',
};

/** Each sport family's heading: the sport names the onboarding chips already read. */
export const SPORT_FAMILY_LABEL_KEY: Record<SportFamily, string> = {
  ride: 'app.sportCycling',
  run: 'app.sportRunning',
  swim: 'app.sportSwimming',
};

/** How far the writer said to trust an anchor, as a row says it. */
export const ANCHOR_CONFIDENCE_LABEL_KEY: Record<AnchorConfidence, string> = {
  low: 'physiology.confidence.low',
  medium: 'physiology.confidence.medium',
  high: 'physiology.confidence.high',
};
