// ABOUTME: Barrel export for Firebase module
// ABOUTME: Re-exports all Firebase utilities for convenient imports

export {
  FIREBASE_NOT_CONFIGURED,
  GOOGLE_SIGNIN_UNAVAILABLE,
  NO_GOOGLE_ID_TOKEN,
  isFirebaseEnabled,
  getFirebaseApp,
  getFirebaseAuth,
  signInWithGoogle,
  signOutFromFirebase,
  subscribeToAuthState,
  getCurrentFirebaseUser,
  getFirebaseIdToken,
  type GoogleSignInResult,
} from './firebase';
