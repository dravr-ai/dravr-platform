// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests that expo-notifications is loaded only in a client with remote notifications
// ABOUTME: In Expo Go on Android the package throws as it loads, so it must never be evaluated there

import { Platform } from 'react-native';

let mockExecutionEnvironment = 'standalone';
jest.mock('expo-constants', () => ({
  __esModule: true,
  default: {
    get executionEnvironment() {
      return mockExecutionEnvironment;
    },
  },
  ExecutionEnvironment: { Bare: 'bare', Standalone: 'standalone', StoreClient: 'storeClient' },
}));

// Counts how often the package is evaluated; in Expo Go on Android the real
// one throws at that moment.
let mockEvaluations = 0;
jest.mock('expo-notifications', () => {
  mockEvaluations += 1;
  return {};
});

describe('expo-notifications loading', () => {
  beforeEach(() => {
    jest.resetModules();
    mockEvaluations = 0;
  });

  afterEach(() => {
    mockExecutionEnvironment = 'standalone';
  });

  it('never evaluates the package in Expo Go on Android', () => {
    mockExecutionEnvironment = 'storeClient';
    const platform = jest.replaceProperty(Platform, 'OS', 'android');
    try {
      const { clientHasRemotePush, loadNotifications } =
        require('../src/services/expoNotifications') as typeof import('../src/services/expoNotifications');

      expect(clientHasRemotePush()).toBe(false);
      expect(loadNotifications()).toBeNull();
      expect(mockEvaluations).toBe(0);
    } finally {
      platform.restore();
    }
  });

  it('loads the package in a store build on Android', () => {
    const platform = jest.replaceProperty(Platform, 'OS', 'android');
    try {
      const { clientHasRemotePush, loadNotifications } =
        require('../src/services/expoNotifications') as typeof import('../src/services/expoNotifications');

      expect(clientHasRemotePush()).toBe(true);
      expect(loadNotifications()).not.toBeNull();
      expect(mockEvaluations).toBe(1);
    } finally {
      platform.restore();
    }
  });

  it('loads the package in Expo Go on iOS, which kept remote notifications', () => {
    mockExecutionEnvironment = 'storeClient';
    const platform = jest.replaceProperty(Platform, 'OS', 'ios');
    try {
      const { loadNotifications } =
        require('../src/services/expoNotifications') as typeof import('../src/services/expoNotifications');

      expect(loadNotifications()).not.toBeNull();
    } finally {
      platform.restore();
    }
  });
});
