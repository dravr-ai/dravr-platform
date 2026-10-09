// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Android app's Play Integrity token on its code exchange: bound to the code's hash, never blocking sign-in
// ABOUTME: Only the native module, the browser sheet and the token request are scripted; the sign-in flow is real (carnet#810)

import { createHash } from 'crypto';
import { Platform } from 'react-native';
import * as WebBrowser from 'expo-web-browser';
import * as AppIntegrity from '@expo/app-integrity';
import { authApi } from '../../services/api';
import type { LoginResponse } from '../../types';
import { signInWithHostedPage } from '../hostedSignIn';

const REDIRECT_URI = 'exp://127.0.0.1:8082/--/auth/callback';
const PROJECT_NUMBER_VAR = 'EXPO_PUBLIC_PLAY_INTEGRITY_CLOUD_PROJECT_NUMBER';

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
  prepareIntegrityTokenProviderAsync: jest.fn(),
  requestIntegrityCheckAsync: jest.fn(),
}));

const integrity = AppIntegrity as jest.Mocked<typeof AppIntegrity>;
const openAuthSession = WebBrowser.openAuthSessionAsync as jest.Mock;

const SESSION = { access_token: 'jwt-token', token_type: 'Bearer' } as LoginResponse;

/** The request hash the server recomputes from the code it redeems. */
function requestHash(code: string): string {
  return createHash('sha256').update(code, 'utf8').digest('base64url');
}

/** The sheet returns `code` to the callback with the opened request's state. */
function redirectWithCode(code: string): void {
  openAuthSession.mockImplementationOnce(async (url: string) => {
    const state = new URL(url).searchParams.get('state') ?? '';
    return { type: 'success', url: `${REDIRECT_URI}?code=${code}&state=${state}` };
  });
}

function refusal(error: string, status = 400): Error {
  return Object.assign(new Error(`Request failed with status code ${status}`), {
    response: { status, data: { error } },
  });
}

describe('Play Integrity on the hosted sign-in', () => {
  let completeSignIn: jest.SpyInstance;
  let os: jest.ReplaceProperty<typeof Platform.OS>;
  const savedProjectNumber = process.env[PROJECT_NUMBER_VAR];

  /**
   * The token provider is prepared once per run for its project, so each case
   * names its own project to start from an unprepared provider.
   */
  function forProject(cloudProjectNumber: string): void {
    process.env[PROJECT_NUMBER_VAR] = cloudProjectNumber;
  }

  beforeEach(() => {
    jest.clearAllMocks();
    mockEnvironment.value = 'standalone';
    os = jest.replaceProperty(Platform, 'OS', 'android');
    integrity.prepareIntegrityTokenProviderAsync.mockResolvedValue(undefined);
    integrity.requestIntegrityCheckAsync.mockImplementation(async (hash) => `play-token(${hash})`);
    completeSignIn = jest.spyOn(authApi, 'completeSignIn').mockResolvedValue(SESSION);
  });

  afterEach(() => {
    completeSignIn.mockRestore();
    os.restore();
    if (savedProjectNumber === undefined) {
      delete process.env[PROJECT_NUMBER_VAR];
    } else {
      process.env[PROJECT_NUMBER_VAR] = savedProjectNumber;
    }
  });

  it('sends a token minted for the base64url SHA-256 of the code', async () => {
    forProject('100000000001');
    redirectWithCode('code-a1');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    const hash = requestHash('code-a1');
    expect(hash).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(integrity.prepareIntegrityTokenProviderAsync).toHaveBeenCalledWith('100000000001');
    expect(integrity.requestIntegrityCheckAsync).toHaveBeenCalledWith(hash);
    expect(completeSignIn).toHaveBeenCalledWith(
      expect.objectContaining({ code: 'code-a1', deviceEvidence: { playIntegrityToken: `play-token(${hash})` } })
    );
    expect(integrity.generateKeyAsync).not.toHaveBeenCalled();
  });

  it('prepares the token provider once per run', async () => {
    forProject('100000000002');
    redirectWithCode('code-a2');
    await signInWithHostedPage();
    redirectWithCode('code-a3');
    await signInWithHostedPage();

    expect(integrity.prepareIntegrityTokenProviderAsync).toHaveBeenCalledTimes(1);
    expect(integrity.requestIntegrityCheckAsync).toHaveBeenCalledTimes(2);
    expect(integrity.requestIntegrityCheckAsync).toHaveBeenLastCalledWith(requestHash('code-a3'));
  });

  it('sends nothing from a build without a cloud project number', async () => {
    delete process.env[PROJECT_NUMBER_VAR];
    redirectWithCode('code-a4');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(integrity.prepareIntegrityTokenProviderAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ deviceEvidence: undefined }));
  });

  it('sends nothing when the cloud project number is not a number', async () => {
    forProject('my-project');
    redirectWithCode('code-a5');

    await signInWithHostedPage();

    expect(integrity.prepareIntegrityTokenProviderAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ deviceEvidence: undefined }));
  });

  it('sends nothing from Expo Go', async () => {
    forProject('100000000003');
    mockEnvironment.value = 'storeClient';
    redirectWithCode('code-a6');

    await signInWithHostedPage();

    expect(integrity.prepareIntegrityTokenProviderAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ deviceEvidence: undefined }));
  });

  it('signs in without evidence when the provider will not prepare, and prepares again next time', async () => {
    forProject('100000000004');
    integrity.prepareIntegrityTokenProviderAsync.mockRejectedValueOnce(new Error('Play services unavailable'));
    redirectWithCode('code-a7');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(integrity.requestIntegrityCheckAsync).not.toHaveBeenCalled();
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ deviceEvidence: undefined }));

    redirectWithCode('code-a8');
    await signInWithHostedPage();

    expect(integrity.prepareIntegrityTokenProviderAsync).toHaveBeenCalledTimes(2);
    expect(completeSignIn).toHaveBeenLastCalledWith(
      expect.objectContaining({ deviceEvidence: { playIntegrityToken: `play-token(${requestHash('code-a8')})` } })
    );
  });

  it('signs in without evidence when Google Play cannot mint a token', async () => {
    forProject('100000000005');
    integrity.requestIntegrityCheckAsync.mockRejectedValueOnce(new Error('NETWORK_ERROR'));
    redirectWithCode('code-a9');

    await expect(signInWithHostedPage()).resolves.toBe(SESSION);

    expect(completeSignIn).toHaveBeenCalledTimes(1);
    expect(completeSignIn).toHaveBeenCalledWith(expect.objectContaining({ deviceEvidence: undefined }));
  });

  it('prepares a fresh provider after a token request fails, as Google asks for an expired one', async () => {
    forProject('100000000007');
    redirectWithCode('code-a11');
    await signInWithHostedPage();
    expect(integrity.prepareIntegrityTokenProviderAsync).toHaveBeenCalledTimes(1);

    integrity.requestIntegrityCheckAsync.mockRejectedValueOnce(
      Object.assign(new Error('The token provider is invalid'), { code: 'ERR_APP_INTEGRITY_PROVIDER_INVALID' })
    );
    redirectWithCode('code-a12');
    await expect(signInWithHostedPage()).resolves.toBe(SESSION);
    expect(completeSignIn).toHaveBeenLastCalledWith(expect.objectContaining({ deviceEvidence: undefined }));

    redirectWithCode('code-a13');
    await signInWithHostedPage();

    expect(integrity.prepareIntegrityTokenProviderAsync).toHaveBeenCalledTimes(2);
    expect(integrity.prepareIntegrityTokenProviderAsync).toHaveBeenLastCalledWith('100000000007');
    expect(completeSignIn).toHaveBeenLastCalledWith(
      expect.objectContaining({ deviceEvidence: { playIntegrityToken: `play-token(${requestHash('code-a13')})` } })
    );
  });

  it('reports a refused token without retrying the exchange', async () => {
    forProject('100000000006');
    const refused = refusal('invalid_client');
    completeSignIn.mockRejectedValueOnce(refused);
    redirectWithCode('code-a10');

    await expect(signInWithHostedPage()).rejects.toBe(refused);

    expect(completeSignIn).toHaveBeenCalledTimes(1);
    expect(integrity.requestIntegrityCheckAsync).toHaveBeenCalledTimes(1);
  });
});
