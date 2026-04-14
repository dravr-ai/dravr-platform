// ABOUTME: Password reset form for entering the 6-digit code and new password
// ABOUTME: Matches Login.tsx glassmorphism design with code entry and password confirmation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { authApi } from '../services/api';
import { Button, Input } from './ui';

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
  const [code, setCode] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    if (code.length !== 6 || !/^\d{6}$/.test(code)) {
      setError('Please enter a valid 6-digit code');
      return;
    }

    if (newPassword.length < 8) {
      setError('Password must be at least 8 characters');
      return;
    }

    if (newPassword !== confirmPassword) {
      setError('Passwords do not match');
      return;
    }

    setIsLoading(true);

    try {
      const response = await authApi.resetPassword(code, newPassword);
      onResetSuccess(response.message || 'Password has been reset successfully. Please sign in.');
    } catch (err: unknown) {
      const apiError = err as { response?: { data?: { message?: string; error?: string }; status?: number } };
      if (apiError.response?.status === 404) {
        setError('Invalid or expired code. Please request a new one.');
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
    <button
      type="button"
      aria-label={showPassword ? 'Hide password' : 'Show password'}
      className="text-on-surface-variant hover:text-on-surface transition-colors"
      onClick={() => setShowPassword(!showPassword)}
    >
      {showPassword ? (
        <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.878 9.878L3 3m6.878 6.878L21 21"
          />
        </svg>
      ) : (
        <svg className="h-5 w-5" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z"
          />
        </svg>
      )}
    </button>
  );

  return (
    <div className="min-h-screen flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-surface">
      <div className="max-w-md w-full">
        <div
          className="rounded-xl border ghost-border overflow-hidden relative"
          style={{
            background: 'rgba(244, 244, 241, 0.8)',
            backdropFilter: 'blur(12px)',
          }}
        >
          <div className="h-1 w-full boreal-hero-gradient" />

          <div className="px-8 py-10 space-y-6">
            <div className="flex flex-col items-center">
              <svg
                className="w-12 h-12 text-pierre-activity"
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
                Enter Reset Code
              </h1>
              <p className="mt-1 text-sm text-on-surface-variant text-center">
                We sent a 6-digit code to <span className="text-on-surface">{email}</span>
              </p>
            </div>

            <form className="space-y-5" onSubmit={handleSubmit}>
              {error && (
                <div
                  role="alert"
                  aria-live="polite"
                  className="bg-red-500/10 border border-red-500/30 text-red-400 px-4 py-3 rounded-lg text-sm"
                >
                  {error}
                </div>
              )}

              <div className="space-y-4">
                <Input
                  id="code"
                  name="code"
                  type="text"
                  inputMode="numeric"
                  label="Reset code"
                  autoComplete="one-time-code"
                  required
                  placeholder="Enter 6-digit code"
                  maxLength={6}
                  value={code}
                  onChange={(e) => setCode(e.target.value.replace(/\D/g, '').slice(0, 6))}
                  variant="dark"
                />

                <Input
                  id="newPassword"
                  name="newPassword"
                  type={showPassword ? 'text' : 'password'}
                  label="New password"
                  autoComplete="new-password"
                  required
                  placeholder="Enter new password (min 8 characters)"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  variant="dark"
                  rightIcon={passwordToggle}
                />

                <Input
                  id="confirmPassword"
                  name="confirmPassword"
                  type={showPassword ? 'text' : 'password'}
                  label="Confirm new password"
                  autoComplete="new-password"
                  required
                  placeholder="Confirm your new password"
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
                {isLoading ? 'Resetting password...' : 'Reset password'}
              </Button>

              <div className="flex items-center justify-between text-sm">
                <button
                  type="button"
                  onClick={onResendCode}
                  className="text-on-surface-variant hover:text-pierre-violet-light transition-colors"
                >
                  Resend code
                </button>
                <button
                  type="button"
                  onClick={onNavigateToLogin}
                  className="text-pierre-violet-light hover:text-pierre-cyan-light font-medium transition-colors"
                >
                  Back to sign in
                </button>
              </div>
            </form>
          </div>
        </div>
      </div>
    </div>
  );
}
