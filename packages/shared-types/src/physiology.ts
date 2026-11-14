// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's physiology wire shape — each anchor per sport family with its range, source, date and confidence
// ABOUTME: With its parser, so a malformed body is an error rather than a threshold printed without its estimate marker

/** The three sport families anchors are held for (carnet#1496). */
export type SportFamily = 'ride' | 'run' | 'swim';

/** The families, in the order the server lists them. */
const SPORT_FAMILIES: readonly SportFamily[] = ['ride', 'run', 'swim'];

/** Which anchor of a sport family a value is. */
export type SportAnchorName =
  | 'ftp'
  | 'critical_power'
  | 'map'
  | 'critical_speed'
  | 'vvo2max'
  | 'threshold_speed'
  | 'max_heart_rate'
  | 'threshold_heart_rate';

/** Every anchor, in the order the server lists a family's anchors. */
const SPORT_ANCHOR_NAMES: readonly SportAnchorName[] = [
  'ftp',
  'critical_power',
  'map',
  'critical_speed',
  'vvo2max',
  'threshold_speed',
  'max_heart_rate',
  'threshold_heart_rate',
];

/** The unit an anchor's value, low and high are in. */
export type AnchorUnit = 'watts' | 'metres_per_second' | 'beats_per_minute';

/** How far to trust an anchor, as the writer stated it; never derived by the platform. */
export type AnchorConfidence = 'low' | 'medium' | 'high';

/** The confidences, lowest first. */
const ANCHOR_CONFIDENCES: readonly AnchorConfidence[] = ['low', 'medium', 'high'];

/** One anchor as it is stored, and the words the server writes for it. */
export interface PhysiologyAnchor {
  anchor: SportAnchorName;
  unit: AnchorUnit;
  /** The stored value, in `unit`. */
  value: number;
  /** The low end of the range it stands for, when one is stated. */
  low: number | null;
  /** The high end of the range it stands for, when one is stated. */
  high: number | null;
  /** Who set it, as stored: `athlete`, a provider, a test. */
  origin: string | null;
  /** The day it was measured, stated or imported, `YYYY-MM-DD`, when known. */
  as_of: string | null;
  /** How it was measured, as stored; for MAP, its definition. */
  method: string | null;
  confidence: AnchorConfidence;
  /**
   * The value as the envelope notice writes it, in the reader's language and
   * units: a range or an estimate at low confidence ("about 245–265 W
   * (estimate)"), a speed as pace per km or mile, per 100 m in the pool.
   */
  value_text: string;
  /** The range it states ("365–395 W"), when `value_text` does not already read as it. */
  range_text: string | null;
  /** Who set it, as the athlete reads it: "your own value", "from intervals.icu". */
  source_text: string;
}

/** One sport family's anchors. */
export interface SportPhysiology {
  sport: SportFamily;
  anchors: PhysiologyAnchor[];
}

/** `GET /api/me/physiology`: each family the athlete holds an anchor in, ride, run, swim. */
export interface PhysiologyResponse {
  sports: SportPhysiology[];
}

const CIVIL_DATE = /^\d{4}-\d{2}-\d{2}$/;
const ANCHOR_UNITS: readonly string[] = ['watts', 'metres_per_second', 'beats_per_minute'];

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

function isOptionalNumber(value: unknown): value is number | null {
  return value === null || isFiniteNumber(value);
}

function isOptionalString(value: unknown): value is string | null {
  return value === null || typeof value === 'string';
}

function parseAnchor(value: unknown): PhysiologyAnchor | null {
  if (!isRecord(value)) return null;
  const { anchor, unit, confidence, as_of: asOf } = value;
  if (
    typeof anchor !== 'string' ||
    !(SPORT_ANCHOR_NAMES as readonly string[]).includes(anchor) ||
    typeof unit !== 'string' ||
    !ANCHOR_UNITS.includes(unit) ||
    typeof confidence !== 'string' ||
    !(ANCHOR_CONFIDENCES as readonly string[]).includes(confidence) ||
    !isFiniteNumber(value.value) ||
    !isOptionalNumber(value.low) ||
    !isOptionalNumber(value.high) ||
    !isOptionalString(value.origin) ||
    !isOptionalString(value.method) ||
    !(asOf === null || (typeof asOf === 'string' && CIVIL_DATE.test(asOf))) ||
    typeof value.value_text !== 'string' ||
    value.value_text === '' ||
    !isOptionalString(value.range_text) ||
    typeof value.source_text !== 'string'
  ) {
    return null;
  }
  return {
    anchor: anchor as SportAnchorName,
    unit: unit as AnchorUnit,
    value: value.value,
    low: value.low,
    high: value.high,
    origin: value.origin,
    as_of: asOf,
    method: value.method,
    confidence: confidence as AnchorConfidence,
    value_text: value.value_text,
    range_text: value.range_text,
    source_text: value.source_text,
  };
}

/**
 * Read a `GET /api/me/physiology` body, or null when it is not one. Every key
 * must be present: an anchor whose words are missing is rejected rather than
 * printed as a bare number, which would drop a low-confidence anchor's
 * estimate marker.
 */
export function parsePhysiologyResponse(body: unknown): PhysiologyResponse | null {
  if (!isRecord(body) || !Array.isArray(body.sports)) return null;
  const sports: SportPhysiology[] = [];
  for (const entry of body.sports) {
    if (
      !isRecord(entry) ||
      typeof entry.sport !== 'string' ||
      !(SPORT_FAMILIES as readonly string[]).includes(entry.sport) ||
      !Array.isArray(entry.anchors)
    ) {
      return null;
    }
    const anchors: PhysiologyAnchor[] = [];
    for (const raw of entry.anchors) {
      const parsed = parseAnchor(raw);
      if (parsed === null) return null;
      anchors.push(parsed);
    }
    sports.push({ sport: entry.sport as SportFamily, anchors });
  }
  return { sports };
}
