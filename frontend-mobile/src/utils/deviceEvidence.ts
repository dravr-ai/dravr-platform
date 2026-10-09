// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The app's integrity evidence for its code exchange: App Attest on iOS, a Play Integrity token on Android
// ABOUTME: Bound to the authorization code; nothing in Expo Go or where the device cannot attest (carnet#810)

import { Platform } from 'react-native';
import Constants, { ExecutionEnvironment } from 'expo-constants';
import * as Crypto from 'expo-crypto';
import * as SecureStore from 'expo-secure-store';
import { base64UrlEncode, type DeviceEvidence } from '@pierre/api-client';

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
 * The Play Integrity module and the Google Cloud project it mints tokens for,
 * on an Android build configured for it; `null` elsewhere.
 *
 * The project number is a build-time value
 * (`EXPO_PUBLIC_PLAY_INTEGRITY_CLOUD_PROJECT_NUMBER`), read on every call: a
 * build without one sends no evidence. Expo Go bundles no `ExpoAppIntegrity`
 * native module, and a token from it would name Expo's package, not Dravr's.
 */
function playIntegrity(): { module: AppIntegrity; cloudProjectNumber: string } | null {
  if (Platform.OS !== 'android') return null;
  if (Constants.executionEnvironment === ExecutionEnvironment.StoreClient) return null;
  const cloudProjectNumber = process.env.EXPO_PUBLIC_PLAY_INTEGRITY_CLOUD_PROJECT_NUMBER ?? '';
  if (!/^\d+$/.test(cloudProjectNumber)) return null;
  try {
    return { module: require('@expo/app-integrity') as AppIntegrity, cloudProjectNumber };
  } catch {
    return null;
  }
}

/**
 * The standard-request token provider, prepared once per app run for its
 * project. Preparing warms Google Play's integrity service, which takes
 * seconds; every later request reuses it. A failed preparation is forgotten so
 * the next sign-in tries again, and so is a provider whose token request
 * failed: Google Play expires a provider kept too long
 * (`INTEGRITY_TOKEN_PROVIDER_INVALID`), and only a new one mints again.
 */
let preparedProvider: { cloudProjectNumber: string; ready: Promise<void> } | null = null;

/** Forget the provider `ready` prepared, unless another has replaced it. */
function forgetProvider(ready: Promise<void>): void {
  if (preparedProvider?.ready === ready) preparedProvider = null;
}

function prepareProvider(module: AppIntegrity, cloudProjectNumber: string): Promise<void> {
  if (preparedProvider?.cloudProjectNumber !== cloudProjectNumber) {
    const ready = module.prepareIntegrityTokenProviderAsync(cloudProjectNumber).catch((error: unknown) => {
      forgetProvider(ready);
      throw error;
    });
    preparedProvider = { cloudProjectNumber, ready };
  }
  return preparedProvider.ready;
}

/**
 * The request hash a Play Integrity token for redeeming `code` is bound to:
 * the unpadded base64url SHA-256 of the code's UTF-8 bytes, which the server
 * recomputes from the code it redeems.
 */
async function playIntegrityRequestHash(code: string): Promise<string> {
  const digest = await Crypto.digest(Crypto.CryptoDigestAlgorithm.SHA256, new TextEncoder().encode(code));
  return base64UrlEncode(new Uint8Array(digest));
}

/**
 * A Play Integrity token bound to `code`, or `null` when Google Play could not
 * mint one (Play services missing or outdated, no network, a provider that
 * would not prepare or has expired). The sign-in then carries no evidence,
 * which the server still accepts while enforcement is off; a failed request
 * drops the provider, so the next sign-in prepares a fresh one.
 */
async function playIntegrityEvidence(code: string): Promise<DeviceEvidenceAttempt | null> {
  const play = playIntegrity();
  if (!play) return null;
  const ready = prepareProvider(play.module, play.cloudProjectNumber);
  try {
    await ready;
  } catch {
    return null;
  }
  try {
    const playIntegrityToken = await play.module.requestIntegrityCheckAsync(await playIntegrityRequestHash(code));
    if (!playIntegrityToken) return null;
    return { evidence: { playIntegrityToken }, accepted: async () => undefined };
  } catch {
    forgetProvider(ready);
    return null;
  }
}

/**
 * Evidence for one code exchange, and what to do with the key once the
 * server has answered.
 */
export interface DeviceEvidenceAttempt {
  evidence: DeviceEvidence;
  /** The exchange succeeded: keep a newly attested key for the next sign-in. */
  accepted(): Promise<void>;
}

/**
 * Evidence that this install of the app is redeeming `code`.
 *
 * On Android, a Play Integrity token bound to the code, with nothing to keep
 * afterwards. On iOS, App Attest: an install with a registered key asserts
 * with it; one without generates a key and has Apple attest it. A key Apple no
 * longer honours (the app was reinstalled, the device restored) is dropped for
 * a fresh one. `null` when the device cannot attest or Apple or Google could
 * not be reached: the server still accepts a sign-in without evidence while
 * enforcement is off.
 *
 * `freshKey` skips the registered key and attests a new one: the server
 * refused the registered key's assertion.
 */
export async function deviceEvidence(
  code: string,
  { freshKey = false }: { freshKey?: boolean } = {}
): Promise<DeviceEvidenceAttempt | null> {
  if (Platform.OS === 'android') return playIntegrityEvidence(code);

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
