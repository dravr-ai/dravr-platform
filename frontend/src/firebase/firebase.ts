// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Firebase SDK configuration and authentication utilities
// ABOUTME: Provides Google Sign-In via Firebase for the Pierre frontend

import { initializeApp, type FirebaseApp } from 'firebase/app';
import {
  getAuth,
  signInWithPopup,
  signInWithRedirect,
  getRedirectResult,
  onAuthStateChanged,
  GoogleAuthProvider,
  signOut,
  type Auth,
  type User,
} from 'firebase/auth';

// Firebase configuration - all values from environment variables
// Set these in frontend/.env or frontend/.env.local:
//   VITE_FIREBASE_API_KEY, VITE_FIREBASE_PROJECT_ID,
//   VITE_FIREBASE_STORAGE_BUCKET, VITE_FIREBASE_MESSAGING_SENDER_ID, VITE_FIREBASE_APP_ID
//
// authDomain is the app's own origin, not firebaseapp.com. The Firebase auth
// handler (/__/auth/*) is reverse-proxied to {projectId}.firebaseapp.com by the
// frontend nginx in production and by the vite dev server locally. Keeping the
// handler same-origin means Firebase stores its pending-redirect state on our
// own origin instead of a third-party site — without that, mobile browsers
// (Safari ITP, Chrome storage partitioning) block the cross-site storage that
// signInWithRedirect/getRedirectResult depend on, so the redirect leg of
// Google sign-in silently never completes on phones.
const firebaseConfig = {
  apiKey: import.meta.env.VITE_FIREBASE_API_KEY,
  authDomain: window.location.host,
  projectId: import.meta.env.VITE_FIREBASE_PROJECT_ID,
  storageBucket: import.meta.env.VITE_FIREBASE_STORAGE_BUCKET,
  messagingSenderId: import.meta.env.VITE_FIREBASE_MESSAGING_SENDER_ID,
  appId: import.meta.env.VITE_FIREBASE_APP_ID,
};

// Check if Firebase is configured
const isFirebaseConfigured = Boolean(
  firebaseConfig.apiKey &&
  firebaseConfig.projectId
);

let app: FirebaseApp | null = null;
let auth: Auth | null = null;

/**
 * Check if Firebase is properly configured via environment variables
 */
export function isFirebaseEnabled(): boolean {
  return isFirebaseConfigured;
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
 * Initiate Google sign-in.
 *
 * Tries the popup flow first: it returns the Firebase ID token directly with
 * no page redirect, and works around Chrome blocking third-party cookies from
 * firebaseapp.com (which breaks redirect-result retrieval on desktop Chrome).
 *
 * In-app browsers (Telegram, Instagram, Messenger) and strict popup blockers
 * reject window.open, so the popup throws. Those environments do allow a
 * full-page redirect, so we fall back to signInWithRedirect — the page then
 * navigates to Google and the result is collected by getGoogleRedirectResult()
 * on the next page load. Returns null in that case to signal "redirecting".
 *
 * Throws if Firebase is not configured.
 */
export async function signInWithGoogle(): Promise<string | null> {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    throw new Error('Google Sign-In is not available. Firebase is not configured.');
  }

  const provider = new GoogleAuthProvider();
  provider.addScope('email');
  provider.addScope('profile');

  try {
    const result = await signInWithPopup(firebaseAuth, provider);
    return result.user.getIdToken();
  } catch (err: unknown) {
    const code = (err as { code?: string }).code;
    if (
      code === 'auth/popup-blocked' ||
      code === 'auth/cancelled-popup-request' ||
      code === 'auth/operation-not-supported-in-environment'
    ) {
      await signInWithRedirect(firebaseAuth, provider);
      // Page is navigating away; the token arrives via getGoogleRedirectResult.
      return null;
    }
    throw err;
  }
}

/**
 * Complete a Google sign-in that used the redirect fallback.
 * Returns the Firebase ID token if the current page load is the return leg of
 * a signInWithRedirect, otherwise null. Safe to call on every page load.
 */
export async function getGoogleRedirectResult(): Promise<string | null> {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    return null;
  }

  const result = await getRedirectResult(firebaseAuth);
  if (!result) {
    return null;
  }
  return result.user.getIdToken();
}

/**
 * Sign out from Firebase
 * No-op if Firebase is not configured
 */
export async function signOutFromFirebase(): Promise<void> {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    return;
  }
  await signOut(firebaseAuth);
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
    // Return no-op unsubscribe if Firebase not configured
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
