// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What Garmin-sourced data contributed to a figure derived from several workouts, as the server says it
// ABOUTME: Decides the line Garmin's brand guidelines owe beneath the figure's title: Garmin as its source, or as a contributing one

/**
 * What Garmin-sourced data contributed to a figure derived from several
 * workouts (carnet#1154): `sole` when every workout behind it carries such
 * data, `part` when some do. The figure's title then carries the matching
 * line; a figure no Garmin workout went into carries none, and its field is
 * null.
 */
export type GarminContribution = 'sole' | 'part';

/** Whether `value` is a contribution the server names. */
export function isGarminContribution(value: unknown): value is GarminContribution {
  return value === 'sole' || value === 'part';
}
