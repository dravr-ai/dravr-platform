// ABOUTME: Barrel export for Firebase module
// ABOUTME: Re-exports all Firebase utilities for convenient imports

export {
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
