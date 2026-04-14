// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { useAsyncAction } from '@pierre/ui-logic';
import { useAuth } from '../hooks/useAuth';
import { useTheme } from '../hooks/useTheme';
import { signInWithGoogle, isFirebaseEnabled } from '../firebase/firebase';
import { Button, Input } from './ui';

// Dravr boreal-palette logo for the login page
function DravrLogo() {
  return (
    <svg width="80" height="80" viewBox="0 0 120 120" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="pg" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#00241a' }} />
          <stop offset="100%" style={{ stopColor: '#0d3b2e' }} />
        </linearGradient>
        <linearGradient id="ag" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#3c6658' }} />
          <stop offset="100%" style={{ stopColor: '#234e40' }} />
        </linearGradient>
        <linearGradient id="ng" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#8f6a2e' }} />
          <stop offset="100%" style={{ stopColor: '#6e5020' }} />
        </linearGradient>
        <linearGradient id="rg" x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" style={{ stopColor: '#5e7a82' }} />
          <stop offset="100%" style={{ stopColor: '#425962' }} />
        </linearGradient>
      </defs>
      <g strokeWidth="2" opacity="0.55" strokeLinecap="round">
        <line x1="40" y1="30" x2="52" y2="42" stroke="url(#ag)" />
        <line x1="52" y1="42" x2="70" y2="35" stroke="url(#ag)" />
        <line x1="52" y1="42" x2="48" y2="55" stroke="url(#pg)" />
        <line x1="48" y1="55" x2="75" y2="52" stroke="url(#ng)" />
        <line x1="48" y1="55" x2="55" y2="72" stroke="url(#pg)" />
        <line x1="55" y1="72" x2="35" y2="85" stroke="url(#rg)" />
        <line x1="55" y1="72" x2="72" y2="82" stroke="url(#rg)" />
      </g>
      <circle cx="40" cy="30" r="7" fill="url(#ag)" />
      <circle cx="52" cy="42" r="5" fill="url(#ag)" />
      <circle cx="70" cy="35" r="3.5" fill="url(#ag)" />
      <circle cx="48" cy="55" r="6" fill="url(#pg)" />
      <circle cx="48" cy="55" r="3" fill="#f9f9f6" opacity="0.9" />
      <circle cx="75" cy="52" r="4.5" fill="url(#ng)" />
      <circle cx="88" cy="60" r="3.5" fill="url(#ng)" />
      <circle cx="55" cy="72" r="5" fill="url(#rg)" />
      <circle cx="35" cy="85" r="4" fill="url(#rg)" />
      <circle cx="72" cy="82" r="4" fill="url(#rg)" />
    </svg>
  );
}

interface LoginProps {
  onNavigateToRegister?: () => void;
  onNavigateToForgotPassword?: () => void;
}

export default function Login({ onNavigateToRegister, onNavigateToForgotPassword }: LoginProps) {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [error, setError] = useState('');
  const [isGoogleLoading, setIsGoogleLoading] = useState(false);
  const { login, loginWithFirebase } = useAuth();
  const { scheme, toggle } = useTheme();

  // Delegate email/password login loading lifecycle to @pierre/ui-logic
  const loginAction = useAsyncAction({
    action: () => login(email, password),
    onError: (err: unknown) => {
      const apiError = err as { response?: { data?: { error?: string } } };
      const errorMsg = apiError.response?.data?.error || 'Login failed';
      if (errorMsg === 'invalid_grant' || errorMsg.includes('Invalid') || errorMsg.includes('credentials')) {
        setError('Invalid email or password');
      } else {
        setError(errorMsg);
      }
    },
    successResetDelay: 0,
    errorResetDelay: 0,
  });

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError('');
    loginAction.execute();
  };

  const handleGoogleSignIn = async () => {
    setIsGoogleLoading(true);
    setError('');

    try {
      // Popup flow: returns ID token directly (no page redirect)
      const idToken = await signInWithGoogle();
      await loginWithFirebase(idToken);
    } catch (err: unknown) {
      const firebaseError = err as { code?: string; message?: string };
      const apiError = err as { response?: { data?: { error?: string } } };

      if (firebaseError.code === 'auth/popup-closed-by-user') {
        // User closed the popup — not an error
      } else if (firebaseError.code === 'auth/network-request-failed') {
        setError('Network error. Please check your connection.');
      } else if (apiError.response?.data?.error) {
        setError(apiError.response.data.error);
      } else {
        setError(firebaseError.message || 'Google sign-in failed');
      }
    } finally {
      setIsGoogleLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex bg-surface text-on-surface">
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
            <DravrLogo />
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
              The Technical Naturalist
            </p>
            <h2
              className="font-display font-semibold text-4xl xl:text-5xl leading-tight"
              style={{ color: '#ffffff' }}
            >
              Fitness intelligence,
              <br />
              rendered in ink.
            </h2>
            <p
              className="text-base leading-relaxed"
              style={{ color: '#a3d0be' }}
            >
              Dravr turns every session, meal, and hour of recovery into a
              single, legible story. For the athletes who want the signal,
              and the coaches who want it cleanly.
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
            <span>Activity</span>
            <span aria-hidden>·</span>
            <span>Nutrition</span>
            <span aria-hidden>·</span>
            <span>Recovery</span>
            <span aria-hidden>·</span>
            <span>Mobility</span>
          </div>
        </div>
      </aside>

      {/* Form column */}
      <main className="flex-1 flex items-center justify-center px-6 py-12 sm:px-12 relative">
        {/* Theme toggle */}
        <button
          type="button"
          onClick={toggle}
          aria-label={scheme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
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

        {/* Mobile-only brand mark (hidden on desktop where the hero shows it) */}
        <div className="lg:hidden absolute top-6 left-6 flex items-center gap-3">
          <DravrLogo />
          <span className="font-display font-semibold text-xl tracking-brand text-on-surface">
            DRAVR
          </span>
        </div>

        <div className="w-full max-w-sm space-y-10">
          <div>
            <h1 className="font-display font-semibold text-3xl text-on-surface">
              Sign in
            </h1>
            <p className="mt-2 text-sm text-on-surface-variant font-label">
              Welcome back. Enter your credentials to continue.
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
                label="Email address"
                autoComplete="email"
                required
                placeholder="you@dravr.ai"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
              <Input
                id="password"
                name="password"
                type={showPassword ? 'text' : 'password'}
                label="Password"
                autoComplete="current-password"
                required
                placeholder="••••••••"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                rightIcon={
                  <button
                    type="button"
                    aria-label={showPassword ? 'Hide password' : 'Show password'}
                    className="text-on-surface-variant hover:text-on-surface transition-colors"
                    onClick={() => setShowPassword(!showPassword)}
                  >
                    {showPassword ? (
                      <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.878 9.878L3 3m6.878 6.878L21 21" />
                      </svg>
                    ) : (
                      <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                      </svg>
                    )}
                  </button>
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
                  Forgot password?
                </button>
              </div>
            )}

            <Button
              type="submit"
              variant="primary"
              loading={loginAction.isLoading}
              className="w-full"
            >
              {loginAction.isLoading ? 'Signing in…' : 'Sign in'}
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
                      or
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
                    {isGoogleLoading ? 'Signing in…' : 'Continue with Google'}
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
                Don&apos;t have an account? Create one
              </button>
            </p>
          )}
        </div>
      </main>
    </div>
  );
}