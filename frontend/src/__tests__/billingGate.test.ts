// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shipped state of the billing gate by evaluating the real features module
// ABOUTME: Billing ships closed: only VITE_BILLING_ENABLED set to exactly 'true' opens it

import { afterEach, describe, expect, it, vi } from 'vitest';

/**
 * SurfaceParity sets the gate through its own mock to render the Dashboard in
 * both states, so it says nothing about which state a build ships in. This
 * evaluates the unmocked module under each value the build can hand it: a
 * default flipped to open, or a looser comparison, fails here.
 */
async function billingEnabledWith(value: string | undefined): Promise<boolean> {
  vi.resetModules();
  vi.stubEnv('VITE_BILLING_ENABLED', value);
  const features = await vi.importActual<typeof import('../constants/features')>('../constants/features');
  return features.BILLING_ENABLED;
}

describe('the billing gate', () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.resetModules();
  });

  it('ships closed: a build that sets nothing has no billing surface', async () => {
    expect(await billingEnabledWith(undefined)).toBe(false);
  });

  it("opens only on exactly 'true'", async () => {
    expect(await billingEnabledWith('true')).toBe(true);
  });

  it.each([['false'], [''], ['1'], ['TRUE'], ['yes'], ['enabled'], [' true']])(
    'stays closed on %j',
    async (value) => {
      expect(await billingEnabledWith(value)).toBe(false);
    },
  );
});
