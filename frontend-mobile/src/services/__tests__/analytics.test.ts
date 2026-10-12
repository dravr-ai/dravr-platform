// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the PostHog distinct id the phone identifies a user with, and that nothing is sent before it is set
// ABOUTME: The same id the server derives with hash_id, so a coach's phone and server events are one PostHog person (carnet#848)

import PostHog from 'posthog-react-native';
import * as Crypto from 'expo-crypto';
import { bootMobileAnalytics, shutdownMobileAnalytics, trackMobile } from '../analytics';

const mockIdentify = jest.fn();
const mockCapture = jest.fn();
const mockReset = jest.fn();

jest.mock('posthog-react-native', () => ({
  __esModule: true,
  default: jest.fn(),
}));

const PostHogClient = PostHog as unknown as jest.Mock;

// The same pair is pinned by `hash_id` in crates/pierre-services/src/analytics.rs,
// by the web app's analytics test and by the shared helper's own test.
const USER_ID = '123e4567-e89b-12d3-a456-426614174000';
const DISTINCT_ID = '986c0dc956dc822b5d8f698661b9eb1ef880786ff9043c16744d2a420e99e9bb';

describe('mobile analytics identity', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    PostHogClient.mockImplementation(() => ({
      identify: mockIdentify,
      capture: mockCapture,
      reset: mockReset,
    }));
    process.env.EXPO_PUBLIC_POSTHOG_API_KEY = 'phc_test_key';
  });

  afterEach(() => {
    shutdownMobileAnalytics();
    delete process.env.EXPO_PUBLIC_POSTHOG_API_KEY;
  });

  it('identifies the user by the lowercase hex SHA-256 of the user UUID, the id the server uses', async () => {
    await bootMobileAnalytics(USER_ID, true);

    expect(mockIdentify).toHaveBeenCalledTimes(1);
    expect(mockIdentify).toHaveBeenCalledWith(DISTINCT_ID);
  });

  it('hashes the UTF-8 bytes of the UUID with expo-crypto SHA-256', async () => {
    await bootMobileAnalytics(USER_ID, true);

    expect(Crypto.digest).toHaveBeenCalledTimes(1);
    const [algorithm, bytes] = (Crypto.digest as jest.Mock).mock.calls[0] as [string, Uint8Array];
    expect(algorithm).toBe(Crypto.CryptoDigestAlgorithm.SHA256);
    // The UUID is ASCII, so its UTF-8 bytes are its character codes.
    expect(Array.from(bytes)).toEqual(Array.from(USER_ID, (ch) => ch.charCodeAt(0)));
  });

  it('creates the client and identifies in one step, so no event goes out anonymously', async () => {
    const booting = bootMobileAnalytics(USER_ID, true);
    // The id is still being hashed: no client exists and a tracked event is dropped.
    trackMobile({ name: 'screen_view', props: { screen: 'early' } });
    expect(PostHogClient).not.toHaveBeenCalled();
    expect(mockCapture).not.toHaveBeenCalled();

    await booting;

    expect(PostHogClient).toHaveBeenCalledTimes(1);
    expect(PostHogClient.mock.invocationCallOrder[0]).toBeLessThan(mockIdentify.mock.invocationCallOrder[0]);
    trackMobile({ name: 'screen_view', props: { screen: 'late' } });
    expect(mockCapture).toHaveBeenCalledTimes(1);
    expect(mockCapture).toHaveBeenCalledWith('screen_view', { screen: 'late' });
  });

  it('does not start PostHog when consent is revoked while the id is being hashed', async () => {
    const booting = bootMobileAnalytics(USER_ID, true);
    shutdownMobileAnalytics();
    await booting;

    expect(PostHogClient).not.toHaveBeenCalled();
    expect(mockIdentify).not.toHaveBeenCalled();
  });

  it('starts PostHog once when a second boot overlaps the first', async () => {
    await Promise.all([bootMobileAnalytics(USER_ID, true), bootMobileAnalytics(USER_ID, true)]);

    expect(PostHogClient).toHaveBeenCalledTimes(1);
    expect(mockIdentify).toHaveBeenCalledTimes(1);
    expect(mockIdentify).toHaveBeenCalledWith(DISTINCT_ID);
  });

  it('does nothing without consent', async () => {
    await bootMobileAnalytics(USER_ID, false);

    expect(PostHogClient).not.toHaveBeenCalled();
    expect(mockIdentify).not.toHaveBeenCalled();
  });

  it('stays off, without throwing, when the native digest fails', async () => {
    (Crypto.digest as jest.Mock).mockRejectedValueOnce(new Error('native digest unavailable'));
    const logged = jest.spyOn(console, 'error').mockImplementation(() => {});

    await expect(bootMobileAnalytics(USER_ID, true)).resolves.toBeUndefined();

    expect(PostHogClient).not.toHaveBeenCalled();
    expect(mockIdentify).not.toHaveBeenCalled();
    expect(logged).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(logged.mock.calls)).not.toContain(USER_ID);
    logged.mockRestore();
  });
});
