// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Firebase SDK configuration and authentication utilities
// ABOUTME: Provides Google Sign-In via Firebase for the Pierre frontend

import { initializeApp, type FirebaseApp } from 'firebase/app';
import {
  getAuth,
  signInWithPopup,
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
// authDomain uses the current origin (self-hosted auth handler via nginx reverse proxy).
// This eliminates third-party cookie issues and manual OAuth redirect URI configuration.
// The nginx proxy routes /__/auth/* to {projectId}.firebaseapp.com transparently.
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
 * Initiate Google sign-in via popup flow.
 * Returns the Firebase ID token directly — no page redirect needed.
 * Uses popup instead of redirect because Chrome blocks third-party cookies
 * from firebaseapp.com, breaking the redirect result retrieval.
 * Throws if Firebase is not configured.
 */
export async function signInWithGoogle(): Promise<string> {
  const firebaseAuth = getFirebaseAuth();
  if (!firebaseAuth) {
    throw new Error('Google Sign-In is not available. Firebase is not configured.');
  }

  const provider = new GoogleAuthProvider();
  provider.addScope('email');
  provider.addScope('profile');

  const result = await signInWithPopup(firebaseAuth, provider);
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
