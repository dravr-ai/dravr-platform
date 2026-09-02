// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState, useEffect } from 'react';
import { useAsyncAction, classifyApiError } from '@pierre/ui-logic';
import { useAuth } from '../hooks/useAuth';
import { useTheme } from '../hooks/useTheme';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
// Only the configured-flag is static. The SDK itself is imported at the point
// of use — on mount to complete a redirect, and on click to start a sign-in —
// so a password login never downloads it.
import { isFirebaseEnabled } from '../firebase/config';
import { Button, Input, RevealButton } from './ui';

import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';

interface LoginProps {
  onNavigateToRegister?: () => void;
  onNavigateToForgotPassword?: () => void;
  prefilledEmail?: string;
}

/**
 * Turn a failed sign-in into a sentence the athlete can act on.
 *
 * The sign-in form is the one screen where a 401 means "wrong password"
 * rather than "your session expired", so it maps that kind itself instead of
 * taking the shared default.
 *
 * Before this, an offline device produced the same message as a rejected
 * password: the request never reached a server, so there was no
 * `response.data.error`, and the code fell through to a hardcoded English
 * t('auth.loginFailed'). An athlete in a tunnel was told their credentials were wrong.
 */
function describeLoginFailure(
  err: unknown,
  online: boolean,
  t: (key: string) => string,
): string {
  const { kind } = classifyApiError(err, { online });
  if (kind === 'credentials' || kind === 'unauthorized' || kind === 'validation') {
    return t('auth.invalidCredentials');
  }
  if (kind === 'offline') {
    return t('errors.offline');
  }
  if (kind === 'network' || kind === 'timeout') {
    return t('errors.network');
  }
  if (kind === 'server') {
    return t('errors.serverError');
  }
  return t('auth.loginFailed');
}

/**
 * The same, for the Google button.
 *
 * Firebase reports a dead network as `auth/network-request-failed` rather than
 * as an absent HTTP response, so that code is folded in before the shared
 * classifier sees the error. The previous version fell back to
 * `firebaseError.message`, which put raw SDK strings ("A network AuthError…")
 * in front of athletes in every locale.
 */
function describeGoogleFailure(
  err: unknown,
  online: boolean,
  t: (key: string) => string,
): string {
  const code = (err as { code?: string }).code;
  if (code === 'auth/network-request-failed') {
    return online ? t('errors.network') : t('errors.offline');
  }
  const { kind } = classifyApiError(err, { online });
  if (kind === 'offline') {
    return t('errors.offline');
  }
  if (kind === 'network' || kind === 'timeout') {
    return t('errors.network');
  }
  if (kind === 'server') {
    return t('errors.serverError');
  }
  return t('auth.googleSignInFailed');
}

export default function Login({ onNavigateToRegister, onNavigateToForgotPassword, prefilledEmail }: LoginProps) {
  const { t } = useTranslation();
  const [email, setEmail] = useState(prefilledEmail ?? '');
  const [password, setPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [error, setError] = useState('');
  const [isGoogleLoading, setIsGoogleLoading] = useState(false);
  const { login, loginWithFirebase } = useAuth();
  const { scheme, toggle } = useTheme();
  const online = useOnlineStatus();

  // Delegate email/password login loading lifecycle to @pierre/ui-logic
  const loginAction = useAsyncAction({
    action: () => login(email, password),
    onError: (err: unknown) => setError(describeLoginFailure(err, online, t)),
    successResetDelay: 0,
    errorResetDelay: 0,
  });

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
        setIsGoogleLoading(true);
        await loginWithFirebase(idToken);
      } catch (err: unknown) {
        if (cancelled) {
          return;
        }
        setError(describeGoogleFailure(err, online, t));
      } finally {
        if (!cancelled) {
          setIsGoogleLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [loginWithFirebase]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    loginAction.execute();
  };

  const handleGoogleSignIn = async () => {
    setIsGoogleLoading(true);
    setError('');

    try {
      // Popup flow returns the ID token directly. Where popups are blocked
      // (in-app browsers), signInWithGoogle falls back to a full-page redirect
      // and returns null — the result is then picked up by the redirect effect
      // above on the next page load, so there is nothing more to do here.
      const { signInWithGoogle } = await import('../firebase/firebase');
      const idToken = await signInWithGoogle();
      if (idToken) {
        await loginWithFirebase(idToken);
        setIsGoogleLoading(false);
      }
      // idToken === null → redirecting away; keep the spinner up until navigation.
    } catch (err: unknown) {
      const firebaseError = err as { code?: string };
      // Closing the popup is a decision, not a failure — say nothing.
      if (firebaseError.code !== 'auth/popup-closed-by-user') {
        setError(describeGoogleFailure(err, online, t));
      }
      setIsGoogleLoading(false);
    }
  };

  return (
    <div className="min-h-dvh flex bg-surface text-on-surface">
      {/*
        Editorial hero column. The hero stays fixed deep-forest in both
        light and dark mode — it's the brand moment, and its text/tokens
        are hardcoded so nothing inverts when the scheme flips.
      */}
      <aside className="hidden lg:flex lg:w-1/2 xl:w-3/5 relative overflow-hidden">
        <div
          className="absolute inset-0"
          style={{ background: 'linear-gradient(145deg, #00241a 0%, #0d3b2e 100%)' }}
        />
        <div className="relative flex flex-col justify-between w-full p-16" style={{ color: '#ffffff' }}>
          {/* Brand mark */}
          <div className="flex items-center gap-4">
            <DravrLogo size={80} />
            <span
              className="font-display font-semibold text-2xl tracking-brand"
              style={{ color: '#a3d0be' }}
            >
              DRAVR
            </span>
          </div>

          {/* Editorial headline */}
          <div className="max-w-xl space-y-6">
            <p
              className="text-xs font-label uppercase"
              style={{
                letterSpacing: '0.18em',
                color: '#a3d0be',
                opacity: 0.85,
              }}
            >
              {t('auth.taglinePersona')}
            </p>
            <h2
              className="font-display font-semibold text-4xl xl:text-5xl leading-tight"
              style={{ color: '#ffffff' }}
            >
              {t('auth.taglineLead')}
              <br />
              {t('auth.taglineTail')}
            </h2>
            <p
              className="text-base leading-relaxed"
              style={{ color: '#a3d0be' }}
            >
              {t('auth.landingBlurb')}
            </p>
          </div>

          {/* Footer band */}
          <div
            className="flex items-center gap-6 text-xs font-label uppercase"
            style={{
              letterSpacing: '0.12em',
              color: '#a3d0be',
              opacity: 0.7,
            }}
          >
            <span>{t('auth.activityLabel')}</span>
            <span aria-hidden>·</span>
            <span>{t('chat.categoryNutrition')}</span>
            <span aria-hidden>·</span>
            <span>{t('chat.categoryRecovery')}</span>
            <span aria-hidden>·</span>
            <span>{t('chat.categoryMobility')}</span>
          </div>
        </div>
      </aside>

      {/* Form column */}
      <main className="flex-1 flex items-center justify-center px-6 py-12 sm:px-12 relative">
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
            <DravrLogo size={80} />
            <span className="font-display font-semibold text-xl tracking-brand text-on-surface">
              DRAVR
            </span>
          </div>

          <div className="text-center lg:text-left">
            <h1 className="font-display font-semibold text-3xl text-on-surface">
              {t('auth.signInButton')}
            </h1>
            <p className="mt-2 text-sm text-on-surface-variant font-label">
              {t('auth.welcomeBackHint')}
            </p>
          </div>

          <form className="space-y-8" onSubmit={handleSubmit}>
            {error && (
              <div
                role="alert"
                aria-live="polite"
                className="px-4 py-3 text-sm rounded-md"
                style={{
                  background: 'var(--color-error-container)',
                  color: 'var(--color-on-error-container)',
                }}
              >
                {error}
              </div>
            )}

            <div className="space-y-6">
              <Input
                id="email"
                name="email"
                type="email"
                label={t('auth.emailAddressLabel')}
                autoComplete="email"
                required
                placeholder="name@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
              <Input
                id="password"
                name="password"
                type={showPassword ? 'text' : 'password'}
                label={t('auth.passwordLabel')}
                autoComplete="current-password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                rightIcon={
                  <RevealButton
  revealed={showPassword}
  onToggle={() => setShowPassword(!showPassword)}
  label={showPassword ? t('auth.hidePassword') : t('auth.showPassword')}
/>
                }
              />
            </div>

            {onNavigateToForgotPassword && (
              <div className="flex justify-end -mt-3">
                <button
                  type="button"
                  onClick={onNavigateToForgotPassword}
                  className="btn-secondary text-xs font-label uppercase"
                  style={{ letterSpacing: '0.08em' }}
                >
                  {t('auth.forgotPasswordLink')}
                </button>
              </div>
            )}

            <Button
              type="submit"
              variant="primary"
              loading={loginAction.isLoading}
              className="w-full"
            >
              {loginAction.isLoading ? t('auth.signingIn') : t('auth.signInAction')}
            </Button>

            {isFirebaseEnabled() && (
              <>
                <div className="relative">
                  <div className="absolute inset-0 flex items-center">
                    <div className="w-full border-t" style={{ borderColor: 'var(--ghost-border)' }} />
                  </div>
                  <div className="relative flex justify-center">
                    <span className="px-3 bg-surface text-xs font-label uppercase text-on-surface-variant"
                          style={{ letterSpacing: '0.12em' }}>
                      {t('auth.orDivider')}
                    </span>
                  </div>
                </div>

                <button
                  type="button"
                  onClick={handleGoogleSignIn}
                  disabled={isGoogleLoading}
                  className="w-full flex items-center justify-center gap-3 px-4 py-2.5 rounded-md bg-surface-container-low hover:bg-surface-container transition-colors disabled:opacity-50 disabled:cursor-not-allowed text-on-surface font-medium"
                  style={{ boxShadow: 'inset 0 0 0 1px var(--ghost-border)' }}
                >
                  {isGoogleLoading ? (
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
                    {isGoogleLoading ? t('auth.signingIn') : t('auth.googleContinueButton')}
                  </span>
                </button>
              </>
            )}
          </form>

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