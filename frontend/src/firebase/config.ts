// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Firebase env config and the "is it configured" flag, with no SDK import
// ABOUTME: Split out so asking the question does not drag 155KB of SDK into the entry chunk

/**
 * Firebase configuration, entirely from environment variables.
 *
 * Set in frontend/.env or frontend/.env.local:
 *   VITE_FIREBASE_API_KEY, VITE_FIREBASE_PROJECT_ID,
 *   VITE_FIREBASE_STORAGE_BUCKET, VITE_FIREBASE_MESSAGING_SENDER_ID,
 *   VITE_FIREBASE_APP_ID
 *
 * authDomain points at Firebase's hosted handler ({projectId}.firebaseapp.com).
 * A same-origin authDomain (window.location.host) was tried to fix mobile
 * redirect sign-in but broke local dev — vite serves http, so Firebase's
 * https://localhost:<port>/__/auth/handler URL fails to load and the popup
 * hangs on "Signing in…". This value is the one that authenticates reliably
 * everywhere.
 */
export const firebaseConfig = {
  apiKey: import.meta.env.VITE_FIREBASE_API_KEY,
  authDomain:
    import.meta.env.VITE_FIREBASE_AUTH_DOMAIN ||
    `${import.meta.env.VITE_FIREBASE_PROJECT_ID}.firebaseapp.com`,
  projectId: import.meta.env.VITE_FIREBASE_PROJECT_ID,
  storageBucket: import.meta.env.VITE_FIREBASE_STORAGE_BUCKET,
  messagingSenderId: import.meta.env.VITE_FIREBASE_MESSAGING_SENDER_ID,
  appId: import.meta.env.VITE_FIREBASE_APP_ID,
};

/**
 * Whether Firebase is configured at all.
 *
 * This lives here, apart from `firebase.ts`, for one reason: the login screen
 * has to ask it on every render to decide whether to draw the Google button,
 * and `firebase.ts` statically imports the SDK. Asking a question about two
 * environment variables therefore put the whole SDK in the entry chunk, on the
 * critical path of every visit, including the majority that sign in with a
 * password and never touch it.
 */
export function isFirebaseEnabled(): boolean {
  return Boolean(firebaseConfig.apiKey && firebaseConfig.projectId);
}
