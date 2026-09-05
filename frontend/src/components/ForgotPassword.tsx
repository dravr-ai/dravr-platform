// ABOUTME: Self-service forgot password form for requesting a reset code
// ABOUTME: Matches Login.tsx glassmorphism design with anti-enumeration messaging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { authApi } from '../services/api';
import { Button, Input } from './ui';
import { useTranslation } from '@pierre/i18n';

interface ForgotPasswordProps {
  onNavigateToLogin: () => void;
  onCodeSent: (email: string) => void;
}

export default function ForgotPassword({ onNavigateToLogin, onCodeSent }: ForgotPasswordProps) {
  const { t } = useTranslation();
  const [email, setEmail] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    if (!email.trim()) {
      setError(t('app.enterEmailAddress'));
      return;
    }

    setIsLoading(true);

    try {
      await authApi.forgotPassword(email.trim());
      onCodeSent(email.trim());
    } catch (err: unknown) {
      const apiError = err as { response?: { data?: { message?: string; error?: string } } };
      setError(
        apiError.response?.data?.message ||
          apiError.response?.data?.error ||
          t('app.somethingWentWrongRetry'),
      );
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface">
      <div className="max-w-md w-full">
        <div
          className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container-lowest"
        >
          <div className="px-8 py-10 space-y-6">
            <div className="flex flex-col items-center">
              <svg
                className="w-12 h-12 text-primary"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z"
                />
              </svg>
              <h1 className="mt-4 text-xl font-bold text-on-surface tracking-tight">
                {t('auth.resetPasswordTitle')}
              </h1>
              <p className="mt-1 text-sm text-on-surface-variant text-center">
                {t('auth.resetRequestHint')}
              </p>
            </div>

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

              <Button
                type="submit"
                variant="gradient"
                loading={isLoading}
                className="w-full"
              >
                {isLoading ? t('auth.sendingCode') : t('auth.sendResetCode')}
              </Button>

              <div className="text-center">
                <button
                  type="button"
                  onClick={onNavigateToLogin}
                  className="text-sm text-primary hover:text-primary-fixed-dim font-medium transition-colors"
                >
                  {t('auth.backToSignIn')}
                </button>
              </div>
            </form>
          </div>
        </div>
      </div>
    </div>
  );
}
