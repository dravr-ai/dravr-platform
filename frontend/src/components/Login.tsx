// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect } from 'react';
import { describeSignInFailure } from '@pierre/ui-logic';
import { useAuth } from '../hooks/useAuth';
import { useTheme } from '../hooks/useTheme';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
// Only SDK-free modules are static: the configured-flag and the failure text.
// The SDK itself is imported at the point of use — on mount below to complete a
// redirect, and on click in GoogleSignInButton to start a sign-in — so an email
// sign-in never downloads it.
import { isFirebaseEnabled } from '../firebase/config';
import { describeGoogleFailure } from '../firebase/googleFailure';
import { Button } from './ui';
import { GoogleSignInButton } from './GoogleSignInButton';

import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';
import { PRODUCT_WORDMARK } from '@pierre/shared-constants';

interface LoginProps {
  onNavigateToRegister?: () => void;
  onNavigateToForgotPassword?: () => void;
}

export default function Login({ onNavigateToRegister, onNavigateToForgotPassword }: LoginProps) {
  const { t } = useTranslation();
  const [googleError, setGoogleError] = useState('');
  const [startFailed, setStartFailed] = useState(false);
  const [isCompletingGoogle, setIsCompletingGoogle] = useState(false);
  const [isRedirecting, setIsRedirecting] = useState(false);
  const { startSignIn, signInFailure, clearSignInFailure, loginWithFirebase } = useAuth();
  const { scheme, toggle } = useTheme();
  const online = useOnlineStatus();

  // One banner for every way a sign-in can come back empty: the hosted
  // sign-in's return leg (refused, or a code that would not redeem), a
  // sign-in that could not be started here, and the Google button.
  const error = signInFailure
    ? describeSignInFailure(signInFailure, { online, t })
    : startFailed
      ? t('auth.loginFailed')
      : googleError;

  const clearErrors = () => {
    clearSignInFailure();
    setStartFailed(false);
    setGoogleError('');
  };

  // Coming back from the hosted sign-in restores this page from the
  // back-forward cache with the spinner still up; take it down.
  useEffect(() => {
    const onPageShow = (event: PageTransitionEvent) => {
      if (event.persisted) setIsRedirecting(false);
    };
    window.addEventListener('pageshow', onPageShow);
    return () => window.removeEventListener('pageshow', onPageShow);
  }, []);

  // Complete a Google sign-in that used the redirect fallback. In-app browsers
  // (Telegram, Instagram, Messenger) block the popup, so signInWithGoogle()
  // redirects to Google there; on the return leg the ID token lands here.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const { getGoogleRedirectResult } = await import('../firebase/firebase');
        const idToken = await getGoogleRedirectResult();
        if (!idToken || cancelled) {
          return;
        }
        setIsCompletingGoogle(true);
        await loginWithFirebase(idToken);
      } catch (err: unknown) {
        if (cancelled) {
          return;
        }
        setGoogleError(describeGoogleFailure(err, online, t));
      } finally {
        if (!cancelled) {
          setIsCompletingGoogle(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [loginWithFirebase]);

  // The password is typed on the server's hosted page, never here
  // (carnet#787): this only opens it. The spinner stays up while the browser
  // navigates away; it comes down only if the page could not be left.
  const handleSignIn = async () => {
    clearErrors();
    setIsRedirecting(true);
    try {
      await startSignIn();
    } catch {
      setStartFailed(true);
      setIsRedirecting(false);
    }
  };

  return (
    <div className="min-h-dvh flex bg-surface text-on-surface pad-safe-top">
      {/*
        Editorial aside — the brand moment. In light it sits on the sage tint
        (`primary-container`) beside a white form sheet: the two paper tones
        the previous pairing used were a half-step apart and read as one
        muddy field. Dark keeps its grey step, where the tint would be a dense
        green wall.

        Because the ground changes between schemes, so does the ink. The tint
        binds `on-primary-container` (9.8:1 here) and the dark grey binds
        `on-surface`, so every text role below is scheme-qualified rather than
        left on the body ink. Body ink on the tint measured 13.9:1 and was
        legible, but a token that carries its own foreground and is paired
        with someone else's is how a system stops being one — the rule is
        worth more than the eleven points of contrast it costs here.
      */}
      <aside className="hidden lg:flex lg:w-1/2 xl:w-3/5 flex-col justify-between border-r ghost-border bg-primary-container p-12 dark:bg-surface-container-low xl:p-16">
        {/* Lockup */}
        <div className="flex items-center gap-3">
          <DravrLogo size={32} />
          <span className="font-display text-xl font-semibold tracking-brand text-primary">
            {PRODUCT_WORDMARK}
          </span>
        </div>

        {/* The mark, large, and the one serif line in the product */}
        <div className="max-w-xl xl:max-w-2xl">
          <DravrLogo size={220} />
          <h2 className="mt-8 font-serif text-4xl italic leading-tight text-on-primary-container dark:text-on-surface xl:text-5xl">
            {t('auth.taglineLead')}
            <br />
            {t('auth.taglineTail')}
          </h2>
        </div>

        {/* The four pillars, in sentence case */}
        <div className="flex items-center gap-3 text-sm text-on-primary-container/85 dark:text-on-surface-variant">
          <span>{t('auth.activityLabel')}</span>
          <span aria-hidden className="text-on-primary-container/50 dark:text-outline">·</span>
          <span>{t('chat.categoryNutrition')}</span>
          <span aria-hidden className="text-on-primary-container/50 dark:text-outline">·</span>
          <span>{t('chat.categoryRecovery')}</span>
          <span aria-hidden className="text-on-primary-container/50 dark:text-outline">·</span>
          <span>{t('chat.categoryMobility')}</span>
        </div>
      </aside>

      {/* Form column */}
      <main className="relative flex flex-1 items-center justify-center bg-surface px-6 py-12 sm:px-12 lg:bg-surface-container-lowest dark:lg:bg-surface">
        {/* Theme toggle */}
        <button
          type="button"
          onClick={toggle}
          aria-label={scheme === 'dark' ? t('auth.switchToLightMode') : t('auth.switchToDarkMode')}
          className="absolute top-6 right-6 p-2 rounded-full text-on-surface-variant hover:text-on-surface hover:bg-surface-container-low transition-colors"
        >
          {scheme === 'dark' ? (
            // Sun icon for "switch to light"
            <svg className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth={1.75} viewBox="0 0 24 24" aria-hidden="true">
              <circle cx="12" cy="12" r="4" />
              <path strokeLinecap="round" d="M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4l1.4-1.4M17 7l1.4-1.4" />
            </svg>
          ) : (
            // Moon icon for "switch to dark"
            <svg className="h-5 w-5" fill="none" stroke="currentColor" strokeWidth={1.75} viewBox="0 0 24 24" aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
            </svg>
          )}
        </button>

        <div className="w-full max-w-sm space-y-8 lg:space-y-10">
          {/* Mobile-only brand mark. Was previously `absolute top-6 left-6`,
              which collided with the vertically-centered t('common.login') heading
              at narrow viewports. Now it sits inline above the heading on
              mobile and centers both the mark and the heading; desktop
              (lg+) keeps the left-aligned form layout because the hero
              column owns the brand moment. */}
          <div className="lg:hidden flex items-center justify-center gap-3">
            <DravrLogo size={40} />
            <span className="font-display font-semibold text-lg tracking-brand text-primary">
              {PRODUCT_WORDMARK}
            </span>
          </div>

          <div className="text-center lg:text-left">
            <h1 className="font-display font-semibold text-3xl text-on-surface">
              {t('auth.signInButton')}
            </h1>
            <p className="mt-2 text-sm text-on-surface-variant">
              {t('auth.welcomeBackHint')}
            </p>
          </div>

          <div className="space-y-8">
            {error && (
              <div
                role="alert"
                aria-live="polite"
                className="px-4 py-3 text-sm rounded-lg bg-error-container text-on-error-container"
              >
                {error}
              </div>
            )}

            <Button size="lg"
              type="button"
              variant="primary"
              loading={isRedirecting}
              onClick={() => void handleSignIn()}
              data-testid="sign-in-button"
              className="w-full"
            >
              {isRedirecting ? t('auth.signingIn') : t('auth.signInWithEmail')}
            </Button>

            {onNavigateToForgotPassword && (
              <div className="flex justify-end -mt-3">
                <button
                  type="button"
                  onClick={onNavigateToForgotPassword}
                  className="btn-tertiary text-sm"
                >
                  {t('auth.forgotPasswordLink')}
                </button>
              </div>
            )}

            {isFirebaseEnabled() && (
              <GoogleSignInButton
                dividerSurface="bg-surface lg:bg-surface-container-lowest dark:lg:bg-surface"
                busy={isCompletingGoogle}
                onStart={clearErrors}
                onError={setGoogleError}
              />
            )}
          </div>

          {onNavigateToRegister && (
            <p className="text-sm text-on-surface-variant text-center">
              <button
                type="button"
                onClick={onNavigateToRegister}
                className="btn-secondary font-medium text-on-surface"
              >
                {t('auth.noAccountCreateOne')}
              </button>
            </p>
          )}
        </div>
      </main>
    </div>
  );
}