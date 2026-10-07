// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the iOS app's App Attest evidence on its code exchange: attest once, assert after, recover a refused key
// ABOUTME: Only the native module, the browser sheet and the token request are scripted; the sign-in flow is real (carnet#810)

import { Platform } from 'react-native';
import * as SecureStore from 'expo-secure-store';
import * as WebBrowser from 'expo-web-browser';
import * as AppIntegrity from '@expo/app-integrity';
import { authApi } from '../../services/api';
import type { LoginResponse } from '../../types';
import { APP_ATTEST_KEY_STORE } from '../appAttest';
import { signInWithHostedPage } from '../hostedSignIn';

const REDIRECT_URI = 'exp://127.0.0.1:8082/--/auth/callback';

jest.mock('expo-linking', () => ({
  createURL: jest.fn((path: string) => `exp://127.0.0.1:8082/--/${path}`),
}));

const mockEnvironment = { value: 'standalone' };
jest.mock('expo-constants', () => ({
  __esModule: true,
  ExecutionEnvironment: { Bare: 'bare', Standalone: 'standalone', StoreClient: 'storeClient' },
  default: {
    get executionEnvironment() {
      return mockEnvironment.value;
    },
  },
}));

jest.mock('@expo/app-integrity', () => ({
  isSupported: true,
  generateKeyAsync: jest.fn(),
  attestKeyAsync: jest.fn(),
  generateAssertionAsync: jest.fn(),
}));

const integrity = AppIntegrity as jest.Mocked<typeof AppIntegrity>;
const secureStore = SecureStore as jest.Mocked<typeof SecureStore>;
const openAuthSession = WebBrowser.openAuthSessionAsync as jest.Mock;

const SESSION = { access_token: 'jwt-token', token_type: 'Bearer' } as LoginResponse;

/** The sheet returns `code` to the callback with the opened request's state. */
function redirectWithCode(code: string): void {
  openAuthSession.mockImplementationOnce(async (url: string) => {
    const state = new URL(url).searchParams.get('state') ?? '';
    return { type: 'success', url: `${REDIRECT_URI}?code=${code}&state=${state}` };
  });
}

/** The install already holds a registered key. */
function registeredKey(keyId: string): void {
  secureStore.getItemAsync.mockImplementation(async (name) => (name === APP_ATTEST_KEY_STORE ? keyId : null));
}

function refusal(error: string, status = 400): Error {
  return Object.assign(new Error(`Request failed with status code ${status}`), {
    response: { status, data: { error } },
  });
}

describe('App Attest on the hosted sign-in', () => {
  let completeSignIn: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    mockEnvironment.value = 'standalone';
    secureStore.getItemAsync.mockResolvedValue(null);
    integrity.generateKeyAsync.mockResolvedValue('new-key');
    integrity.attestKeyAsync.mockImplementation(async (keyId, code) => `attestation(${keyId},${code})`);
    integrity.generateAssertionAsync.mockImplementation(async (keyId, code) => `assertion(${keyId},${code})`);
    completeSignIn = jest.spyOn(authApi, 'completeSignIn').mockResolvedValue(SESSION);
  });

  afterEach(() => {
    completeSignIn.mockRestore();
  });

  it('attests a new key over the code on an install’s first sign-in, and keeps it once accepted', async () => {
    redirectWithCode('code-1');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(integrity.attestKeyAsync).toHaveBeenCalledWith('new-key', 'code-1');
    expect(completeSignIn).toHaveBeenCalledWith(
      expect.objectContaining({ appAttest: { keyId: 'new-key', attestation: 'attestation(new-key,code-1)' } })
    );
    expect(secureStore.setItemAsync).toHaveBeenCalledWith(APP_ATTEST_KEY_STORE, 'new-key');
  });

  it('asserts with the registered key over the code on every later sign-in', async () => {
    registeredKey('kept-key');
    redirectWithCode('code-2');

    await signInWithHostedPage();

    expect(integrity.generateKeyAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(
      expect.objectContaining({ appAttest: { keyId: 'kept-key', assertion: 'assertion(kept-key,code-2)' } })
    );
    expect(secureStore.setItemAsync).not.toHaveBeenCalled();
  });

  it('drops a refused key and redeems the same code with a freshly attested one', async () => {
    registeredKey('lost-key');
    completeSignIn.mockRejectedValueOnce(refusal('invalid_client'));
    redirectWithCode('code-3');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(secureStore.deleteItemAsync).toHaveBeenCalledWith(APP_ATTEST_KEY_STORE);
    expect(completeSignIn).toHaveBeenCalledTimes(2);
    expect(completeSignIn).toHaveBeenLastCalledWith(
      expect.objectContaining({ code: 'code-3', appAttest: { keyId: 'new-key', attestation: 'attestation(new-key,code-3)' } })
    );
    expect(secureStore.setItemAsync).toHaveBeenCalledWith(APP_ATTEST_KEY_STORE, 'new-key');
  });

  it('keeps no key the server refused, and reports the refusal', async () => {
    const refused = refusal('invalid_client');
    completeSignIn.mockRejectedValueOnce(refused);
    redirectWithCode('code-4');

    await expect(signInWithHostedPage()).rejects.toBe(refused);

    expect(completeSignIn).toHaveBeenCalledTimes(1);
    expect(secureStore.setItemAsync).not.toHaveBeenCalled();
  });

  it('does not retry a refusal that is not about the evidence', async () => {
    registeredKey('kept-key');
    const refused = refusal('invalid_grant');
    completeSignIn.mockRejectedValueOnce(refused);
    redirectWithCode('code-5');

    await expect(signInWithHostedPage()).rejects.toBe(refused);

    expect(completeSignIn).toHaveBeenCalledTimes(1);
    expect(secureStore.deleteItemAsync).not.toHaveBeenCalled();
  });

  it('does not retry a refused code, which answers invalid_client as a 401', async () => {
    registeredKey('kept-key');
    const refused = refusal('invalid_client', 401);
    completeSignIn.mockRejectedValueOnce(refused);
    redirectWithCode('code-10');

    await expect(signInWithHostedPage()).rejects.toBe(refused);

    expect(completeSignIn).toHaveBeenCalledTimes(1);
    expect(secureStore.deleteItemAsync).not.toHaveBeenCalled();
  });

  it('replaces a key Apple no longer honours before redeeming', async () => {
    registeredKey('reinstalled-key');
    integrity.generateAssertionAsync.mockRejectedValueOnce(new Error('Invalid key provided'));
    redirectWithCode('code-6');

    await signInWithHostedPage();

    expect(secureStore.deleteItemAsync).toHaveBeenCalledWith(APP_ATTEST_KEY_STORE);
    expect(completeSignIn).toHaveBeenCalledWith(
      expect.objectContaining({ appAttest: { keyId: 'new-key', attestation: 'attestation(new-key,code-6)' } })
    );
  });

  it('redeems without evidence when Apple cannot be reached', async () => {
    integrity.attestKeyAsync.mockRejectedValueOnce(new Error('Server unavailable'));
    redirectWithCode('code-7');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ appAttest: undefined }));
  });

  it('sends no evidence from Expo Go', async () => {
    mockEnvironment.value = 'storeClient';
    redirectWithCode('code-8');

    await signInWithHostedPage();

    expect(integrity.generateKeyAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ appAttest: undefined }));
  });

  it('sends no evidence from Android', async () => {
    const os = jest.replaceProperty(Platform, 'OS', 'android');
    redirectWithCode('code-9');

    await signInWithHostedPage();

    os.restore();
    expect(integrity.generateKeyAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ appAttest: undefined }));
  });
});
