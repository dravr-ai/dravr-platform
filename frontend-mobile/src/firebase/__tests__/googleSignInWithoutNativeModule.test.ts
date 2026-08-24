// ABOUTME: Pins that the app still loads on a binary with no native Google Sign-In module
// ABOUTME: The package throws while it evaluates, so the gate must hide the button instead

// A binary built without `RNGoogleSignin` — Expo Go is one — throws while the
// Google Sign-In package evaluates, not when a function on it is called. This
// factory reproduces that. It lives in its own file because a top-level import
// of the package would resolve it before the factory could stand in.
jest.mock('@react-native-google-signin/google-signin', () => {
  throw new Error(
    "TurboModuleRegistry.getEnforcing(...): 'RNGoogleSignin' could not be found. " +
      'Verify that a module by this name is registered in the native binary.'
  );
});

// babel-preset-expo rewrites every `process.env.EXPO_PUBLIC_*` read into
// `require('expo/virtual/env').env.*`, so this is the seam the module reads
// through. Both Firebase and the iOS Google client are configured here, so the
// missing native module is the only thing left that can turn the gate off.
const mockEnv: Record<string, string | undefined> = {
  EXPO_PUBLIC_FIREBASE_API_KEY: 'test-api-key',
  EXPO_PUBLIC_FIREBASE_AUTH_DOMAIN: 'dravr-dev-8d4a3.firebaseapp.com',
  EXPO_PUBLIC_FIREBASE_PROJECT_ID: 'dravr-dev-8d4a3',
  EXPO_PUBLIC_GOOGLE_IOS_CLIENT_ID:
    '629001562818-fqu15igkvlj6jt1ftusktilq7rpg5imn.apps.googleusercontent.com',
  EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID:
    '629001562818-aruetllrbhotqnjvoq7tsssbrfgpf576.apps.googleusercontent.com',
};
jest.mock('expo/virtual/env', () => ({ env: mockEnv }));

jest.mock('firebase/app', () => ({
  initializeApp: jest.fn(() => ({ name: 'test-app' })),
}));

jest.mock('firebase/auth', () => ({
  getAuth: jest.fn(() => ({ currentUser: null })),
  onAuthStateChanged: jest.fn(),
  signOut: jest.fn().mockResolvedValue(undefined),
  signInWithCredential: jest.fn(),
  GoogleAuthProvider: { credential: jest.fn() },
}));

/**
 * Load the module after the mock factories above are in place. A static import
 * would run before `mockEnv` is initialised, since jest hoists the factories
 * but not the binding they close over.
 */
function loadFirebaseModule(): typeof import('../firebase') {
  return require('../firebase') as typeof import('../firebase');
}

describe('Google Sign-In on a binary without the native module', () => {
  it('still exports a usable module rather than throwing at import', () => {
    const { isFirebaseEnabled, signInWithGoogle } = loadFirebaseModule();

    // A module-scope import of the package took every route down with it:
    // AuthContext pulls this file into the whole router tree, so the app
    // rendered blank and every route reported a missing default export.
    expect(typeof isFirebaseEnabled).toBe('function');
    expect(typeof signInWithGoogle).toBe('function');
  });

  it('hides the Google button', () => {
    const { isFirebaseEnabled } = loadFirebaseModule();

    expect(isFirebaseEnabled()).toBe(false);
  });

  it('refuses the sign-in flow with a reason naming the binary', async () => {
    const { signInWithGoogle } = loadFirebaseModule();

    await expect(signInWithGoogle()).rejects.toThrow(
      'This binary has no native Google Sign-In module.'
    );
  });

  it('signs out of Firebase without reaching for the native session', async () => {
    const { signOutFromFirebase } = loadFirebaseModule();

    await expect(signOutFromFirebase()).resolves.toBeUndefined();
  });
});
