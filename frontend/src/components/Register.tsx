// ABOUTME: User registration page for new account creation
// ABOUTME: Matches Login.tsx design with Pierre brand aesthetic
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { authApi } from '../services/api';
import { Button, Input, RevealButton } from './ui';

import { DravrLogo } from './DravrLogo';
import { useTranslation } from '@pierre/i18n';

interface RegisterProps {
  onNavigateToLogin: () => void;
  onRegistrationSuccess: (message: string, email: string) => void;
}

export default function Register({ onNavigateToLogin, onRegistrationSuccess }: RegisterProps) {
  const { t } = useTranslation();
  const [displayName, setDisplayName] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');

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

    try {
      const response = await authApi.register({ email, password, display_name: displayName || undefined });
      onRegistrationSuccess(response.message, email);
    } catch (err: unknown) {
      const apiError = err as { response?: { data?: { message?: string; error?: string } } };
      setError(apiError.response?.data?.message || apiError.response?.data?.error || t('app.registrationFailedRetry'));
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface">
      <div className="max-w-md w-full">
        {/* Card with glassmorphism and gradient accent bar */}
        <div
          className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container"
        >
          {/* Gradient accent bar at top */}
          <div className="h-1 w-full boreal-hero-gradient" />

          <div className="px-8 py-10 space-y-6">
            {/* Logo and brand */}
            <div className="flex flex-col items-center">
              <DravrLogo size={80} />
              <h1 className="mt-4 text-xl font-bold text-on-surface tracking-tight">
                {t('auth.createAccountTitle')}
              </h1>
              <p className="mt-1 text-sm text-on-surface-variant">
                {t('auth.joinDravr')}
              </p>
            </div>

            {/* Registration form */}
            <form className="space-y-5" onSubmit={handleSubmit}>
              {error && (
                <div className="bg-error/10 border border-error/30 text-error px-4 py-3 rounded-lg text-sm">
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

              <Button
                type="submit"
                variant="gradient"
                loading={isLoading}
                className="w-full shadow-ambient"
              >
                {isLoading ? t('auth.creatingAccount') : t('auth.createAccountButton')}
              </Button>

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
          </div>
        </div>
      </div>
    </div>
  );
}
