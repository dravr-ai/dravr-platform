// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves NativeWind's eager component registration no longer reads React Native's deprecated SafeAreaView
// ABOUTME: That read warned on every bundle load (carnet#356); our own imports are held by the lint rule instead

/**
 * carnet#356 counted 63 "SafeAreaView has been deprecated" warnings across two
 * Android runs. None of our screens imported it: react-native-css-interop
 * 0.2.1, which NativeWind loads at startup, registered `react_native.SafeAreaView`
 * and so tripped React Native's deprecation getter on every bundle load. 0.2.7
 * registers react-native-safe-area-context's instead. A NativeWind or
 * css-interop bump that brings the old read back turns this red.
 */
const DEPRECATION = 'SafeAreaView has been deprecated';

function deprecationWarnings(load: () => void): string[] {
  const warn = jest.spyOn(console, 'warn').mockImplementation(() => undefined);
  try {
    jest.isolateModules(load);
    return warn.mock.calls.map((args) => args.map(String).join(' ')).filter((line) => line.includes(DEPRECATION));
  } finally {
    warn.mockRestore();
  }
}

describe("React Native's deprecated SafeAreaView", () => {
  it("is not read by css-interop's eager component registration", () => {
    const warnings = deprecationWarnings(() => {
      require('react-native-css-interop/dist/runtime/components');
    });
    expect(warnings).toEqual([]);
  });

  it('warns when it is read, so the test above can see a regression', () => {
    const warnings = deprecationWarnings(() => {
      const reactNative = require('react-native') as Record<string, unknown>;
      expect(reactNative.SafeAreaView).toBeDefined();
    });
    expect(warnings).toHaveLength(1);
  });
});
