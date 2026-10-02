// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the shipped state of the mobile billing gate by evaluating the real features module
// ABOUTME: Billing ships closed: only EXPO_PUBLIC_BILLING_ENABLED set to exactly 'true' opens it

/**
 * SettingsPaneParity sets the gate through its own mock to render both states,
 * so it says nothing about which state a build ships in. This evaluates the
 * unmocked module under each value the build can hand it: a default flipped to
 * open, or a looser comparison, fails here. The web gate is pinned the same way
 * in frontend/src/__tests__/billingGate.test.ts.
 */
const VARIABLE = 'EXPO_PUBLIC_BILLING_ENABLED';
const original = process.env[VARIABLE];

function billingEnabledWith(value: string | undefined): boolean {
  if (value === undefined) {
    delete process.env[VARIABLE];
  } else {
    process.env[VARIABLE] = value;
  }
  let enabled: boolean | undefined;
  jest.isolateModules(() => {
    enabled = jest.requireActual<typeof import('../src/constants/features')>('../src/constants/features')
      .BILLING_ENABLED;
  });
  if (enabled === undefined) throw new Error('the features module exported no BILLING_ENABLED');
  return enabled;
}

describe('the mobile billing gate', () => {
  afterEach(() => {
    if (original === undefined) {
      delete process.env[VARIABLE];
    } else {
      process.env[VARIABLE] = original;
    }
  });

  it('ships closed: a build that sets nothing has no billing surface', () => {
    expect(billingEnabledWith(undefined)).toBe(false);
  });

  it("opens only on exactly 'true'", () => {
    expect(billingEnabledWith('true')).toBe(true);
  });

  it.each([['false'], [''], ['1'], ['TRUE'], ['yes'], ['enabled'], [' true']])('stays closed on %j', (value) => {
    expect(billingEnabledWith(value)).toBe(false);
  });
});
