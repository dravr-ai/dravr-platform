// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The iOS app's App Attest evidence for its code exchange: attest a Secure Enclave key once, assert with it after
// ABOUTME: Signed over the authorization code; nothing on Android, in Expo Go or where the device cannot attest (carnet#810)

import { Platform } from 'react-native';
import Constants, { ExecutionEnvironment } from 'expo-constants';
import * as SecureStore from 'expo-secure-store';
import type { AppAttestEvidence } from '@pierre/api-client';

/** Where the install's attested key id is kept, once the server registered it. */
export const APP_ATTEST_KEY_STORE = 'app_attest_key_id';

type AppIntegrity = typeof import('@expo/app-integrity');

/**
 * The App Attest module, on an iOS build that can attest; `null` elsewhere.
 *
 * Loaded lazily: Expo Go bundles no `ExpoAppIntegrity` native module, and an
 * attestation from Expo Go would name Expo's app rather than Dravr's anyway.
 * The simulator reports `isSupported: false`.
 */
function appIntegrity(): AppIntegrity | null {
  if (Platform.OS !== 'ios') return null;
  if (Constants.executionEnvironment === ExecutionEnvironment.StoreClient) return null;
  try {
    const module = require('@expo/app-integrity') as AppIntegrity;
    return module.isSupported ? module : null;
  } catch {
    return null;
  }
}

/**
 * Evidence for one code exchange, and what to do with the key once the
 * server has answered.
 */
export interface AppAttestAttempt {
  evidence: AppAttestEvidence;
  /** The exchange succeeded: keep a newly attested key for the next sign-in. */
  accepted(): Promise<void>;
}

/**
 * Evidence that this install of the app is redeeming `code`.
 *
 * An install with a registered key asserts with it; one without generates a
 * key and has Apple attest it. A key Apple no longer honours (the app was
 * reinstalled, the device restored) is dropped for a fresh one. `null` when
 * the device cannot attest or Apple could not be reached: the server still
 * accepts a sign-in without evidence until the Android app attests too.
 *
 * `freshKey` skips the registered key and attests a new one: the server
 * refused the registered key's assertion.
 */
export async function appAttestEvidence(
  code: string,
  { freshKey = false }: { freshKey?: boolean } = {}
): Promise<AppAttestAttempt | null> {
  const integrity = appIntegrity();
  if (!integrity) return null;

  const keyId = freshKey ? null : await SecureStore.getItemAsync(APP_ATTEST_KEY_STORE);
  if (keyId) {
    try {
      const assertion = await integrity.generateAssertionAsync(keyId, code);
      return { evidence: { keyId, assertion }, accepted: async () => undefined };
    } catch {
      await forgetAppAttestKey();
    }
  }

  try {
    const newKeyId = await integrity.generateKeyAsync();
    const attestation = await integrity.attestKeyAsync(newKeyId, code);
    return {
      evidence: { keyId: newKeyId, attestation },
      accepted: () => SecureStore.setItemAsync(APP_ATTEST_KEY_STORE, newKeyId),
    };
  } catch {
    return null;
  }
}

/** Drop the install's key: the server refused it, or Apple no longer honours it. */
export async function forgetAppAttestKey(): Promise<void> {
  await SecureStore.deleteItemAsync(APP_ATTEST_KEY_STORE);
}
