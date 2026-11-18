// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Names Settings › Physiology in the reader's language — each anchor's label, each family's heading, each confidence, the edit form
// ABOUTME: The server writes an anchor's value and source; both clients render the rest, and the coach's view, through these keys

import type { AnchorConfidence, AnchorKind, SportAnchorName, SportFamily } from '@pierre/shared-types';

/** The corpus key for each line of the physiology pane that is not an anchor's own words. */
export const PHYSIOLOGY_KEY = {
  title: 'physiology.title',
  blurb: 'physiology.blurbEditable',
  empty: 'physiology.emptyEditable',
  loadFailed: 'physiology.loadFailed',
  asOf: 'physiology.asOf',
  noDate: 'physiology.noDate',
  range: 'physiology.range',
} as const;

/**
 * The words of the form that sets one anchor (carnet#1561), the same on the
 * athlete's pane and the coach's view of one athlete. `edit` and `cancel`
 * are the app's common words.
 */
export const PHYSIOLOGY_EDIT_KEY = {
  edit: 'common.edit',
  cancel: 'common.cancel',
  save: 'common.save',
  add: 'physiology.edit.add',
  titleEdit: 'physiology.edit.titleEdit',
  sport: 'physiology.edit.sport',
  anchor: 'physiology.edit.anchor',
  value: 'physiology.edit.value',
  paceUnit: 'physiology.edit.paceUnit',
  rangeLow: 'physiology.edit.rangeLow',
  rangeHigh: 'physiology.edit.rangeHigh',
  kind: 'physiology.edit.kind',
  confidence: 'physiology.edit.confidence',
  confidenceHint: 'physiology.edit.confidenceHint',
  method: 'physiology.edit.method',
  measuredOn: 'physiology.edit.measuredOn',
  saving: 'physiology.edit.saving',
  saveFailed: 'physiology.edit.saveFailed',
  invalidNumber: 'physiology.edit.invalidNumber',
  invalidPace: 'physiology.edit.invalidPace',
} as const;

/**
 * The most characters the physiology writer keeps in an anchor's method
 * (`MEASUREMENT_SOURCE_MAX_CHARS` in `pierre-tool-runtime`): the form stops
 * typing there rather than sending a method the server refuses.
 */
export const PHYSIOLOGY_METHOD_MAX_CHARS = 80;

/** How the form names each way an anchor was found. */
export const ANCHOR_KIND_LABEL_KEY: Record<AnchorKind, string> = {
  measured: 'physiology.edit.kindMeasured',
  estimated: 'physiology.edit.kindEstimated',
};

/** The coach's view of one coached athlete's thresholds, from Group info (carnet#1561). */
export const COACH_PHYSIOLOGY_KEY = {
  open: 'humanCoach.athleteCard.thresholds',
  title: 'humanCoach.physiology.title',
  blurb: 'humanCoach.physiology.blurb',
  empty: 'humanCoach.physiology.empty',
  loadFailed: 'humanCoach.physiology.loadFailed',
  consentRevoked: 'humanCoach.physiology.consentRevoked',
  saved: 'humanCoach.physiology.saved',
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
