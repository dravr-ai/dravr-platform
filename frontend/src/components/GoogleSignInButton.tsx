// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "or" and "Continue with Google", shared by the sign-in and create-account forms
// ABOUTME: A Google sign-in creates the account when none exists, so both forms offer it

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { useAuth } from '../hooks/useAuth';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
import { describeGoogleFailure } from '../firebase/googleFailure';

interface GoogleSignInButtonProps {
  /** The ground the "or" label sits on, so it masks the rule behind it. */
  dividerSurface: string;
  /** Called as a sign-in starts, so the form can clear its banner. */
  onStart: () => void;
  /** Called with the banner text when the sign-in fails. */
  onError: (message: string) => void;
  /** Spins the button while a sign-in started elsewhere completes: Login's redirect return leg. */
  busy?: boolean;
}

export function GoogleSignInButton({ dividerSurface, onStart, onError, busy = false }: GoogleSignInButtonProps) {
  const { t } = useTranslation();
  const { loginWithFirebase } = useAuth();
  const online = useOnlineStatus();
  const [isStarting, setIsStarting] = useState(false);
  const isLoading = busy || isStarting;

  const handleClick = async () => {
    setIsStarting(true);
    onStart();

    try {
      // Popup flow returns the ID token directly. Where popups are blocked
      // (in-app browsers), signInWithGoogle falls back to a full-page redirect
      // and returns null. The app comes back on '/', where Login's redirect
      // effect picks the result up, so there is nothing more to do here.
      const { signInWithGoogle } = await import('../firebase/firebase');
      const idToken = await signInWithGoogle();
      if (idToken) {
        await loginWithFirebase(idToken);
        setIsStarting(false);
      }
      // idToken === null → redirecting away; keep the spinner up until navigation.
    } catch (err: unknown) {
      const firebaseError = err as { code?: string };
      // Closing the popup is a decision, not a failure — say nothing.
      if (firebaseError.code !== 'auth/popup-closed-by-user') {
        onError(describeGoogleFailure(err, online, t));
      }
      setIsStarting(false);
    }
  };

  return (
    <>
      <div className="relative">
        <div className="absolute inset-0 flex items-center">
          <div className="w-full border-t" style={{ borderColor: 'var(--ghost-border)' }} />
        </div>
        <div className="relative flex justify-center">
          <span className={`px-3 text-xs text-on-surface-variant ${dividerSurface}`}>
            {t('auth.orDivider')}
          </span>
        </div>
      </div>

      <button
        type="button"
        onClick={handleClick}
        disabled={isLoading}
        className="w-full flex items-center justify-center gap-3 px-4 py-2.5 rounded-lg border ghost-border-strong bg-transparent hover:bg-surface-container-low transition-colors disabled:opacity-50 disabled:cursor-not-allowed text-on-surface font-medium"
      >
        {isLoading ? (
          <div className="pierre-spinner w-5 h-5"></div>
        ) : (
          <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z" fill="#4285F4"/>
            <path d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z" fill="#34A853"/>
            <path d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z" fill="#FBBC05"/>
            <path d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z" fill="#EA4335"/>
          </svg>
        )}
        <span>
          {isLoading ? t('auth.signingIn') : t('auth.googleContinueButton')}
        </span>
      </button>
    </>
  );
}
