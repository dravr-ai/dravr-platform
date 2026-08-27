// ABOUTME: Covers the native Google Sign-In path that mints a Firebase session token
// ABOUTME: Pins the gate that hides the button and the cancel/no-token branches

import { NativeModules, Platform } from 'react-native';
import {
  GoogleSignin,
  type SignInResponse,
} from '@react-native-google-signin/google-signin';
import { signInWithCredential, GoogleAuthProvider } from 'firebase/auth';

// babel-preset-expo rewrites every `process.env.EXPO_PUBLIC_*` read into
// `require('expo/virtual/env').env.*`, so this is the seam the module actually
// reads through — assigning to process.env in a test would never be observed.
const mockEnv: Record<string, string | undefined> = {};
jest.mock('expo/virtual/env', () => ({ env: mockEnv }));

// The module resolves the package only on a binary that registers the native
// `RNGoogleSignin` module — Expo Go does not, which is the whole point of the
// guard in ../firebase. These tests cover the path on a binary that DOES carry
// it, so the double has to be registered; without it the module correctly reports
// the flow as unavailable and never reaches the mocked package below. Registered
// on the real `NativeModules` rather than by mocking `react-native` wholesale,
// which would evaluate every lazy getter in its index and blow up.
(NativeModules as Record<string, unknown>).RNGoogleSignin = {};

jest.mock('@react-native-google-signin/google-signin', () => ({
  GoogleSignin: {
    configure: jest.fn(),
    signIn: jest.fn(),
    signOut: jest.fn().mockResolvedValue(null),
    hasPlayServices: jest.fn().mockResolvedValue(true),
  },
  isSuccessResponse: (response: { type: string }) => response.type === 'success',
}));

jest.mock('firebase/app', () => ({
  initializeApp: jest.fn(() => ({ name: 'test-app' })),
}));

jest.mock('firebase/auth', () => ({
  getAuth: jest.fn(() => ({ currentUser: null })),
  onAuthStateChanged: jest.fn(),
  signOut: jest.fn().mockResolvedValue(undefined),
  signInWithCredential: jest.fn(),
  GoogleAuthProvider: { credential: jest.fn((token: string) => ({ token })) },
}));

const mockSignIn = GoogleSignin.signIn as jest.Mock;
const mockConfigure = GoogleSignin.configure as jest.Mock;
const mockSignInWithCredential = signInWithCredential as jest.Mock;
const mockCredential = GoogleAuthProvider.credential as unknown as jest.Mock;

const IOS_CLIENT_ID = '629001562818-fqu15igkvlj6jt1ftusktilq7rpg5imn.apps.googleusercontent.com';
const WEB_CLIENT_ID = '629001562818-aruetllrbhotqnjvoq7tsssbrfgpf576.apps.googleusercontent.com';

/** Re-require the module so it re-reads process.env at load time. */
function loadFirebaseModule(): typeof import('../firebase') {
  let mod: typeof import('../firebase') | undefined;
  jest.isolateModules(() => {
    mod = jest.requireActual<typeof import('../firebase')>('../firebase');
  });
  if (!mod) {
    throw new Error('failed to load ../firebase in isolation');
  }
  return mod;
}

function successResponse(idToken: string | null): SignInResponse {
  return {
    type: 'success',
    data: {
      idToken,
      serverAuthCode: null,
      scopes: [],
      user: {
        id: 'google-uid',
        name: 'Phil Tremblay',
        email: 'phil@dravr.ai',
        photo: null,
        familyName: 'Tremblay',
        givenName: 'Phil',
      },
    },
  } as SignInResponse;
}

describe('native Google Sign-In', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    Platform.OS = 'ios';
    for (const key of Object.keys(mockEnv)) {
      delete mockEnv[key];
    }
    mockEnv.EXPO_PUBLIC_FIREBASE_API_KEY = 'test-api-key';
    mockEnv.EXPO_PUBLIC_FIREBASE_AUTH_DOMAIN = 'dravr-dev-8d4a3.firebaseapp.com';
    mockEnv.EXPO_PUBLIC_FIREBASE_PROJECT_ID = 'dravr-dev-8d4a3';
    mockEnv.EXPO_PUBLIC_GOOGLE_IOS_CLIENT_ID = IOS_CLIENT_ID;
    mockEnv.EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID = WEB_CLIENT_ID;
  });

  it('exchanges the Google id token for a Firebase session token', async () => {
    mockSignIn.mockResolvedValue(successResponse('google-id-token'));
    mockSignInWithCredential.mockResolvedValue({
      user: {
        email: 'phil@dravr.ai',
        displayName: 'Phil Tremblay',
        getIdToken: jest.fn().mockResolvedValue('firebase-session-token'),
      },
    });

    const { signInWithGoogle } = loadFirebaseModule();
    const result = await signInWithGoogle();

    // The backend validates the Firebase-minted token, never the raw Google one.
    expect(result).toEqual({
      idToken: 'firebase-session-token',
      email: 'phil@dravr.ai',
      displayName: 'Phil Tremblay',
    });
    expect(mockCredential).toHaveBeenCalledWith('google-id-token');
  });

  it('configures the native SDK with the iOS and web client ids', async () => {
    mockSignIn.mockResolvedValue(successResponse('google-id-token'));
    mockSignInWithCredential.mockResolvedValue({
      user: {
        email: 'phil@dravr.ai',
        displayName: null,
        getIdToken: jest.fn().mockResolvedValue('firebase-session-token'),
      },
    });

    const { signInWithGoogle } = loadFirebaseModule();
    await signInWithGoogle();

    expect(mockConfigure).toHaveBeenCalledWith({
      iosClientId: IOS_CLIENT_ID,
      webClientId: WEB_CLIENT_ID,
      scopes: ['email', 'profile'],
    });
  });

  it('returns null when the user dismisses the native sheet', async () => {
    mockSignIn.mockResolvedValue({ type: 'cancelled', data: null });

    const { signInWithGoogle } = loadFirebaseModule();

    await expect(signInWithGoogle()).resolves.toBeNull();
    expect(mockSignInWithCredential).not.toHaveBeenCalled();
  });

  it('throws rather than signing in when Google returns no id token', async () => {
    mockSignIn.mockResolvedValue(successResponse(null));

    const { signInWithGoogle } = loadFirebaseModule();

    await expect(signInWithGoogle()).rejects.toThrow('No ID token received from Google');
    expect(mockSignInWithCredential).not.toHaveBeenCalled();
  });

  it('is enabled on iOS once the iOS client id is present', async () => {
    const { isFirebaseEnabled } = loadFirebaseModule();
    expect(isFirebaseEnabled()).toBe(true);
  });

  it('is disabled on a binary with no native Google Sign-In module', async () => {
    // Expo Go. Everything else is configured, so the missing native module is the
    // only reason the flow is unavailable — and the package must never be required,
    // because its first statement throws and Metro reports that to LogBox before it
    // rethrows, putting a red overlay over the login screen on every render.
    const registry = NativeModules as Record<string, unknown>;
    delete registry.RNGoogleSignin;
    try {
      const { isFirebaseEnabled } = loadFirebaseModule();
      expect(isFirebaseEnabled()).toBe(false);
    } finally {
      registry.RNGoogleSignin = {};
    }
  });

  it('is disabled when the Firebase api key is missing', async () => {
    delete mockEnv.EXPO_PUBLIC_FIREBASE_API_KEY;

    const { isFirebaseEnabled } = loadFirebaseModule();
    expect(isFirebaseEnabled()).toBe(false);
  });

  it('is disabled on Android while no Android client id is configured', async () => {
    Platform.OS = 'android';

    const { isFirebaseEnabled } = loadFirebaseModule();
    expect(isFirebaseEnabled()).toBe(false);
  });

  it('clears the cached Google account on sign out', async () => {
    mockSignIn.mockResolvedValue(successResponse('google-id-token'));
    mockSignInWithCredential.mockResolvedValue({
      user: {
        email: 'phil@dravr.ai',
        displayName: null,
        getIdToken: jest.fn().mockResolvedValue('firebase-session-token'),
      },
    });

    const { signInWithGoogle, signOutFromFirebase } = loadFirebaseModule();
    await signInWithGoogle();
    await signOutFromFirebase();

    expect(GoogleSignin.signOut).toHaveBeenCalled();
  });
});
