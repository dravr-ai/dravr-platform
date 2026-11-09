// ABOUTME: The import window's hashes: each source's own, the source list for one the window has no hash for
// ABOUTME: Pins that a hash names its source back, so a card's export link and the window agree
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect } from 'vitest';
import { GARMIN_ARCHIVE_SOURCE, STRAVA_ARCHIVE_SOURCE } from '@pierre/shared-types';
import { importSourceForHash, importWindowHash } from '../openImportWindow';

describe('import window hashes', () => {
  it('names each source by its own hash, and reads it back', () => {
    expect(importWindowHash(GARMIN_ARCHIVE_SOURCE)).toBe('#garmin');
    expect(importWindowHash(STRAVA_ARCHIVE_SOURCE)).toBe('#strava');
    expect(importSourceForHash('#garmin')).toBe(GARMIN_ARCHIVE_SOURCE);
  });

  it('opens the source list for a source with no hash, and reads no source from an unknown hash', () => {
    expect(importWindowHash('fit_zip_archive')).toBe('');
    expect(importSourceForHash('#elsewhere')).toBeNull();
    expect(importSourceForHash('')).toBeNull();
  });
});
