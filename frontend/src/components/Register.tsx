// ABOUTME: User registration page for new account creation
// ABOUTME: Reads the sign-up config first: a paused notice when closed, a Turnstile check when keyed; takes an optional signup code
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { authApi } from '../services/api';
import { isFirebaseEnabled } from '../firebase/config';
import { Button, Input, RevealButton } from './ui';
import { GoogleSignInButton } from './GoogleSignInButton';
import { TurnstileWidget, type TurnstileWidgetHandle } from './TurnstileWidget';
import { QUERY_KEYS } from '../constants/queryKeys';

import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';
import { describeRegistration, isInvalidSignupCode } from '@pierre/ui-logic';

interface RegisterProps {
  onNavigateToLogin: () => void;
  onRegistrationSuccess: (message: string, email: string) => void;
  /** A signup code the create-account link carried (`/signup?code=…`). */
  initialSignupCode?: string;
}

export default function Register({ onNavigateToLogin, onRegistrationSuccess, initialSignupCode }: RegisterProps) {
  const { t } = useTranslation();
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');
  const [turnstileToken, setTurnstileToken] = useState<string | null>(null);
  // A signup code approves the account at once (carnet#1128). Most people have
  // none, so the field stays behind a link unless the link carried one.
  const [signupCode, setSignupCode] = useState(initialSignupCode ?? '');
  const [showSignupCode, setShowSignupCode] = useState(Boolean(initialSignupCode));
  const turnstileRef = useRef<TurnstileWidgetHandle>(null);

  // Whether sign-up is open and which bot check to render. A server that does
  // not answer this keeps the plain form: it is the one that enforces the
  // guards, and the form must not vanish because a read failed.
  const signupConfigQuery = useQuery({
    queryKey: QUERY_KEYS.signup.config(),
    queryFn: () => authApi.getSignupConfig(),
    retry: 1,
  });
  const signupPaused = signupConfigQuery.data?.open === false;
  const turnstileSiteKey = signupConfigQuery.data?.turnstile_site_key ?? null;
  const awaitingTurnstile = turnstileSiteKey !== null && turnstileToken === null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    // Validate passwords match
    if (password !== confirmPassword) {
      setError(t('app.passwordsDoNotMatch'));
      return;
    }

    // Validate password strength
    if (password.length < 8) {
      setError(t('app.passwordTooShort'));
      return;
    }

    setIsLoading(true);

    const trimmedCode = signupCode.trim();
    try {
      const response = await authApi.register({
        email,
        password,
        display_name: displayName || undefined,
        turnstile_token: turnstileToken ?? undefined,
        ...(trimmedCode ? { signup_code: trimmedCode } : {}),
      });
      onRegistrationSuccess(describeRegistration(response, t), email);
    } catch (err: unknown) {
      // The server's sentence covers each guard: paused or past the day's
      // ceiling (429 QuotaExceeded), too many sign-ups from this network
      // (429 RateLimitExceeded), the human check failed (403), the signup
      // code is not one (400, nothing created: the field stays to fix it).
      const apiError = err as { response?: { data?: { message?: string; error?: string } } };
      setError(
        isInvalidSignupCode(err)
          ? t('auth.signupCodeInvalid')
          : apiError.response?.data?.message ||
              apiError.response?.data?.error ||
              t('app.registrationFailedRetry')
      );
      // A Turnstile token is single-use: the next attempt needs a fresh one.
      turnstileRef.current?.reset();
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface pad-safe-top">
      <div className="max-w-md w-full">
        {/* The one card language: white, a hairline, no strip */}
        <div
          className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container-lowest"
        >
          <div className="px-8 py-10 space-y-6">
            {/* Logo and brand */}
            <div className="flex flex-col items-center">
              <DravrLogo size={64} />
              <h1 className="mt-4 font-display text-2xl font-semibold text-on-surface">
                {t('auth.createAccountTitle')}
              </h1>
              <p className="mt-1 text-sm text-on-surface-variant">
                {t('auth.joinDravr')}
              </p>
            </div>

            {signupPaused ? (
              <div className="space-y-5">
                <div
                  role="status"
                  className="bg-surface-container-low border ghost-border text-on-surface px-4 py-3 rounded-lg text-sm"
                  data-testid="signup-paused"
                >
                  {t('auth.signupPaused')}
                </div>
                <div className="text-center">
                  <button
                    type="button"
                    onClick={onNavigateToLogin}
                    className="text-sm text-primary hover:text-primary-fixed-dim font-medium transition-colors"
                  >
                    {t('auth.haveAccountSignIn')}
                  </button>
                </div>
              </div>
            ) : (
            /* Registration form */
            <form className="space-y-5" onSubmit={handleSubmit}>
              {error && (
                <div
                  role="alert"
                  aria-live="polite"
                  className="bg-error/10 border border-error/30 text-error px-4 py-3 rounded-lg text-sm"
                >
                  {error}
                </div>
              )}

              <div className="space-y-4">
                <Input
                  id="displayName"
                  name="displayName"
                  type="text"
                  label={t('auth.displayNameLabel')}
                  autoComplete="name"
                  placeholder={t('auth.displayNamePlaceholder')}
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                  variant="dark"
                />

                <Input
                  id="email"
                  name="email"
                  type="email"
                  label={t('auth.emailAddressLabel')}
                  autoComplete="email"
                  required
                  placeholder={t('auth.emailPlaceholder')}
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  variant="dark"
                />

                <Input
                  id="password"
                  name="password"
                  type={showPassword ? 'text' : 'password'}
                  label={t('auth.passwordLabel')}
                  autoComplete="new-password"
                  required
                  placeholder={t('auth.newPasswordPlaceholderSignUp')}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  variant="dark"
                  rightIcon={
                    <RevealButton
  revealed={showPassword}
  onToggle={() => setShowPassword(!showPassword)}
  label={showPassword ? t('auth.hidePassword') : t('auth.showPassword')}
/>
                  }
                />

                <Input
                  id="confirmPassword"
                  name="confirmPassword"
                  type={showPassword ? 'text' : 'password'}
                  label={t('auth.confirmPasswordLabel')}
                  autoComplete="new-password"
                  required
                  placeholder={t('auth.confirmPasswordPlaceholder')}
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  variant="dark"
                />
              </div>

              {showSignupCode ? (
                <Input
                  id="signupCode"
                  name="signupCode"
                  type="text"
                  label={t('auth.signupCodeLabel')}
                  autoComplete="off"
                  autoCapitalize="characters"
                  spellCheck={false}
                  placeholder={t('auth.signupCodePlaceholder')}
                  helpText={t('auth.signupCodeHint')}
                  value={signupCode}
                  onChange={(e) => setSignupCode(e.target.value)}
                  variant="dark"
                />
              ) : (
                <button
                  type="button"
                  onClick={() => setShowSignupCode(true)}
                  className="text-sm text-primary hover:text-primary-fixed-dim font-medium transition-colors"
                  data-testid="signup-code-reveal"
                >
                  {t('auth.haveSignupCode')}
                </button>
              )}

              {turnstileSiteKey && (
                <TurnstileWidget
                  ref={turnstileRef}
                  siteKey={turnstileSiteKey}
                  onToken={setTurnstileToken}
                  onLoadError={() => setError(t('errors.network'))}
                  className="flex justify-center"
                />
              )}

              <Button size="lg"
                type="submit"
                variant="gradient"
                loading={isLoading}
                disabled={signupConfigQuery.isLoading || awaitingTurnstile}
                className="w-full"
              >
                {isLoading ? t('auth.creatingAccount') : t('auth.createAccountButton')}
              </Button>

              {/* A Google sign-in creates the account, so it is a way to sign up too. */}
              {isFirebaseEnabled() && (
                <GoogleSignInButton
                  dividerSurface="bg-surface-container-lowest"
                  onStart={() => setError('')}
                  onError={setError}
                />
              )}

              {/* Link to login */}
              <div className="text-center">
                <button
                  type="button"
                  onClick={onNavigateToLogin}
                  className="text-sm text-primary hover:text-primary-fixed-dim font-medium transition-colors"
                >
                  {t('auth.haveAccountSignIn')}
                </button>
              </div>
            </form>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
