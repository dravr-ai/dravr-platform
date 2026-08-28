// ABOUTME: Password reset form for entering the emailed reset code and new password
// ABOUTME: Matches Login.tsx glassmorphism design with code entry and password confirmation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { authApi } from '../services/api';
import { Button, Input, RevealButton } from './ui';
import { useTranslation } from '@pierre/i18n';

interface ResetPasswordProps {
  email: string;
  onNavigateToLogin: () => void;
  onResetSuccess: (message: string) => void;
  onResendCode: () => void;
}

export default function ResetPassword({
  email,
  onNavigateToLogin,
  onResetSuccess,
  onResendCode,
}: ResetPasswordProps) {
  const { t } = useTranslation();
  const [code, setCode] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    const trimmedCode = code.trim();
    if (!/^[A-Za-z0-9]+\.[A-Za-z0-9]+$/.test(trimmedCode)) {
      setError(t('app.enterResetCodeFromEmail'));
      return;
    }

    if (newPassword.length < 8) {
      setError(t('app.passwordTooShort'));
      return;
    }

    if (newPassword !== confirmPassword) {
      setError(t('app.passwordsDoNotMatch'));
      return;
    }

    setIsLoading(true);

    try {
      const response = await authApi.resetPassword(trimmedCode, newPassword);
      onResetSuccess(response.message || t('app.passwordResetDone'));
    } catch (err: unknown) {
      const apiError = err as { response?: { data?: { message?: string; error?: string }; status?: number } };
      if (apiError.response?.status === 404) {
        setError(t('app.codeInvalidOrExpired'));
      } else {
        setError(
          apiError.response?.data?.message ||
            apiError.response?.data?.error ||
            'Reset failed. Please try again.',
        );
      }
    } finally {
      setIsLoading(false);
    }
  };

  const passwordToggle = (
    <RevealButton
  revealed={showPassword}
  onToggle={() => setShowPassword(!showPassword)}
  label={showPassword ? t('auth.hidePassword') : t('auth.showPassword')}
/>
  );

  return (
    <div className="min-h-dvh flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface">
      <div className="max-w-md w-full">
        <div
          className="rounded-xl border ghost-border overflow-hidden relative bg-surface-container"
        >
          <div className="h-1 w-full boreal-hero-gradient" />

          <div className="px-8 py-10 space-y-6">
            <div className="flex flex-col items-center">
              <svg
                className="w-12 h-12 text-activity"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
                aria-hidden="true"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z"
                />
              </svg>
              <h1 className="mt-4 text-xl font-bold text-on-surface tracking-tight">
                {t('auth.enterResetCodeTitle')}
              </h1>
              <p className="mt-1 text-sm text-on-surface-variant text-center">
                {t('auth.resetCodeSentTo')} <span className="text-on-surface">{email}</span>
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

              <div className="space-y-4">
                <Input
                  id="code"
                  name="code"
                  type="text"
                  label={t('auth.resetCodeLabel')}
                  autoComplete="one-time-code"
                  autoCapitalize="none"
                  autoCorrect="off"
                  spellCheck={false}
                  required
                  placeholder={t('auth.resetCodePlaceholder')}
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  variant="dark"
                />

                <Input
                  id="newPassword"
                  name="newPassword"
                  type={showPassword ? 'text' : 'password'}
                  label={t('auth.newPasswordLabel')}
                  autoComplete="new-password"
                  required
                  placeholder={t('auth.newPasswordPlaceholderReset')}
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  variant="dark"
                  rightIcon={passwordToggle}
                />

                <Input
                  id="confirmPassword"
                  name="confirmPassword"
                  type={showPassword ? 'text' : 'password'}
                  label={t('auth.confirmNewPasswordLabel')}
                  autoComplete="new-password"
                  required
                  placeholder={t('auth.confirmNewPasswordPlaceholder')}
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
                {isLoading ? t('auth.resettingPassword') : t('auth.resetPasswordButton')}
              </Button>

              <div className="flex items-center justify-between text-sm">
                <button
                  type="button"
                  onClick={onResendCode}
                  className="text-on-surface-variant hover:text-primary transition-colors"
                >
                  {t('auth.resendCode')}
                </button>
                <button
                  type="button"
                  onClick={onNavigateToLogin}
                  className="text-primary hover:text-primary-fixed-dim font-medium transition-colors"
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
