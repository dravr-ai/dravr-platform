// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that starting the catalogue prints nothing to the console — no i18next vendor notice in any app
// ABOUTME: i18next logs a Locize advert on init unless told not to, and a browser bundle has no NODE_ENV to hush it

import { afterEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '../src/config';

describe('initI18n', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('prints no vendor notice to the console when it starts', async () => {
    // The production web bundle runs without `process`, so i18next's own
    // NODE_ENV check never silences the notice there; this test process is
    // not production either, which is what lets it see the same thing.
    const info = vi.spyOn(console, 'info').mockImplementation(() => undefined);
    const log = vi.spyOn(console, 'log').mockImplementation(() => undefined);

    await initI18n({ persistLocale: () => Promise.resolve() });

    const printed = [...info.mock.calls, ...log.mock.calls].map((call) => call.map(String).join(' '));
    expect(printed.filter((line) => /i18next|locize/i.test(line))).toEqual([]);
  });
});
