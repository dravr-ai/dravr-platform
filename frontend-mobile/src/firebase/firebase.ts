// ABOUTME: Firebase SDK configuration and authentication utilities for React Native
// ABOUTME: Google credentials come from the native Google Sign-In SDK; Firebase mints the session token

import { initializeApp, type FirebaseApp } from 'firebase/app';
import {
  getAuth,
  onAuthStateChanged,
  signOut,
  GoogleAuthProvider,
  signInWithCredential,
  type Auth,
  type User,
} from 'firebase/auth';
import {
  GoogleSignin,
  isSuccessResponse,
} from '@react-native-google-signin/google-signin';
import { Platform } from 'react-native';

// Firebase configuration - all values from environment variables
// Set these in your .env file with EXPO_PUBLIC_ prefix
const firebaseConfig = {
  apiKey: process.env.EXPO_PUBLIC_FIREBASE_API_KEY,
  authDomain: process.env.EXPO_PUBLIC_FIREBASE_AUTH_DOMAIN,
  projectId: process.env.EXPO_PUBLIC_FIREBASE_PROJECT_ID,
  storageBucket: process.env.EXPO_PUBLIC_FIREBASE_STORAGE_BUCKET,
  messagingSenderId: process.env.EXPO_PUBLIC_FIREBASE_MESSAGING_SENDER_ID,
  appId: process.env.EXPO_PUBLIC_FIREBASE_APP_ID,
};

// Google OAuth client IDs - the native SDK picks the one matching the platform.
// The iOS client is bound to the `ai.dravr.app` bundle id; its reversed form is
// registered as a URL scheme by the google-signin config plugin in app.config.js.
const googleClientIds = {
  iosClientId: process.env.EXPO_PUBLIC_GOOGLE_IOS_CLIENT_ID,
  // LIMITATION(registre#92): `androidClientId` has no Android OAuth client behind it,
  // so the gate below fails closed on Android and hides the Google button there.
  androidClientId: process.env.EXPO_PUBLIC_GOOGLE_ANDROID_CLIENT_ID,
  webClientId: process.env.EXPO_PUBLIC_GOOGLE_WEB_CLIENT_ID,
};

// Check if Firebase is configured
const isFirebaseConfigured = Boolean(
  firebaseConfig.apiKey &&
  firebaseConfig.authDomain &&
  firebaseConfig.projectId
);

// Check if Google OAuth is configured for the current platform
// iOS requires iosClientId, Android requires androidClientId, web uses webClientId
function isPlatformGoogleOAuthConfigured(): boolean {
  if (Platform.OS === 'ios') {
    return Boolean(googleClientIds.iosClientId);
  }
  if (Platform.OS === 'android') {
    return Boolean(googleClientIds.androidClientId);
  }
  // Web/other platforms can use webClientId
  return Boolean(googleClientIds.webClientId);
}

let app: FirebaseApp | null = null;
let auth: Auth | null = null;
let googleSigninConfigured = false;

/**
 * The identity returned by a completed Google sign-in, ready for the backend.
 * `idToken` is the Firebase-minted token, not the raw Google one.
 */
export type GoogleSignInResult = {
  idToken: string;
  email: string;
  displayName: string | null;
};

/**
 * Check if Firebase is properly configured via environment variables
 * Returns true only if both Firebase and platform-specific Google OAuth are configured
 */
export function isFirebaseEnabled(): boolean {
  return isFirebaseConfigured && isPlatformGoogleOAuthConfigured();
}

/**
 * Initialize Firebase app (lazy initialization)
 * Returns null if Firebase is not configured
 */
export function getFirebaseApp(): FirebaseApp | null {
  if (!isFirebaseConfigured) {
    return null;
  }
  if (!app) {
    app = initializeApp(firebaseConfig);
  }
  return app;
}

/**
 * Get Firebase Auth instance
 * Returns null if Firebase is not configured
 */
export function getFirebaseAuth(): Auth | null {
  if (!isFirebaseConfigured) {
    return null;
  }
  if (!auth) {
    const firebaseApp = getFirebaseApp();
    if (!firebaseApp) {
      return null;
    }
    auth = getAuth(firebaseApp);
  }
  return auth;
}

/**
 * Hand the native SDK its client IDs once per app run.
 * Only the client IDs configured for this platform are passed, so a missing
 * Android client cannot be mistaken for an empty-string one.
 */
function configureGoogleSignin(): void {
  if (googleSigninConfigured) {
    return;
  }
  GoogleSignin.configure({
    ...(googleClientIds.iosClientId ? { iosClientId: googleClientIds.iosClientId } : {}),
    ...(googleClientIds.webClientId ? { webClientId: googleClientIds.webClientId } : {}),
    scopes: ['email', 'profile'],
  });
  googleSigninConfigured = true;
}

/**
 * Run the native Google sign-in flow and exchange the result for a Firebase session.
 * Returns null when the user dismisses the native sheet.
 */
export async function signInWithGoogle(): Promise<GoogleSignInResult | null> {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    throw new Error('Google Sign-In is not available. Firebase is not configured.');
  }

  configureGoogleSignin();

  if (Platform.OS === 'android') {
    await GoogleSignin.hasPlayServices({ showPlayServicesUpdateDialog: true });
  }

  const response = await GoogleSignin.signIn();
  if (!isSuccessResponse(response)) {
    return null;
  }

  const googleIdToken = response.data.idToken;
  if (!googleIdToken) {
    throw new Error('No ID token received from Google');
  }

  // Create Firebase credential from Google ID token
  const credential = GoogleAuthProvider.credential(googleIdToken);

  // Sign in to Firebase with the credential
  const userCredential = await signInWithCredential(firebaseAuth, credential);

  // Get Firebase ID token for backend authentication
  const firebaseIdToken = await userCredential.user.getIdToken();

  return {
    idToken: firebaseIdToken,
    email: userCredential.user.email || '',
    displayName: userCredential.user.displayName,
  };
}

/**
 * Sign out from Firebase and drop the cached Google account.
 * Clearing the native session makes the next sign-in show the account picker
 * again rather than silently reusing the last account.
 */
export async function signOutFromFirebase(): Promise<void> {
  const firebaseAuth = getFirebaseAuth();
  if (firebaseAuth) {
    await signOut(firebaseAuth);
  }
  if (googleSigninConfigured) {
    await GoogleSignin.signOut();
  }
}

/**
 * Subscribe to Firebase auth state changes
 * Returns an unsubscribe function (no-op if Firebase not configured)
 */
export function subscribeToAuthState(
  callback: (user: User | null) => void
): () => void {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    return () => {};
  }
  return onAuthStateChanged(firebaseAuth, callback);
}

/**
 * Get the current Firebase user (if signed in)
 * Returns null if Firebase not configured
 */
export function getCurrentFirebaseUser(): User | null {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    return null;
  }
  return firebaseAuth.currentUser;
}

/**
 * Get ID token for current user
 * Returns null if Firebase not configured or no user
 */
export async function getFirebaseIdToken(): Promise<string | null> {
  const user = getCurrentFirebaseUser();
  if (!user) {
    return null;
  }
  return user.getIdToken();
}
