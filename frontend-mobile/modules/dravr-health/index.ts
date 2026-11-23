// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The JS face of the local dravr-health Expo module — HealthKit on iOS, Health Connect on Android, read-only
// ABOUTME: Null wherever the native module is absent (Expo Go, web, a build without it); every answer is `unknown` until the sync service checks it

import { requireOptionalNativeModule } from 'expo';

/**
 * What the native side offers. Answers are typed `unknown` on purpose: they
 * cross a bridge, and `src/services/deviceHealth` checks every field before a
 * byte of it reaches the server.
 */
export interface DravrHealthNative {
  /** Whether the OS store exists on this device (HealthKit data available; Health Connect installed). */
  isAvailable(): Promise<boolean>;
  /**
   * Ask to read the types named (HealthKit identifiers, or Health Connect
   * record class names); nothing is ever asked for writing. Resolves when the
   * prompt completed: HealthKit never says what was declined.
   */
  requestAuthorization(types: string[]): Promise<boolean>;
  /**
   * One page of one type's changes since `anchor` (`null`: the first read,
   * back to `since`, an RFC 3339 instant): `{ workouts, categories,
   * quantities, states_of_mind, deleted, anchor, more }`.
   */
  readChanges(type: string, anchor: string | null, since: string, limit: number): Promise<unknown>;
  /** Every sleep sample overlapping `[from, to]`, as category samples. */
  readSleep(from: string, to: string): Promise<unknown>;
  /** Every sample of one quantity type starting in `[from, to)`, as quantity samples. */
  readQuantities(type: string, from: string, to: string): Promise<unknown>;
  /** The store's merged steps and active energy for each local `YYYY-MM-DD` date that has any. */
  readDays(dates: string[]): Promise<unknown>;
}

export const DravrHealth: DravrHealthNative | null = requireOptionalNativeModule<DravrHealthNative>('DravrHealth');
