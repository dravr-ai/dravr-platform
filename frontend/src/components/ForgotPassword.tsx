// ABOUTME: Self-service forgot password form for requesting a reset code
// ABOUTME: Matches Login.tsx glassmorphism design with anti-enumeration messaging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { authApi } from '../services/api';
import { Button, Input } from './ui';

interface ForgotPasswordProps {
  onNavigateToLogin: () => void;
  onCodeSent: (email: string) => void;
}

export default function ForgotPassword({ onNavigateToLogin, onCodeSent }: ForgotPasswordProps) {
  const [email, setEmail] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState('');

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError('');

    if (!email.trim()) {
      setError('Please enter your email address');
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
          'Something went wrong. Please try again.',
      );
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center py-12 px-4 sm:px-6 lg:px-8 bg-pierre-dark">
      <div className="max-w-md w-full">
        <div
          className="rounded-xl border border-white/10 overflow-hidden relative"
          style={{
            background: 'rgba(30, 30, 46, 0.8)',
            backdropFilter: 'blur(12px)',
          }}
        >
          <div className="h-1 w-full bg-gradient-pierre-horizontal" />

          <div className="px-8 py-10 space-y-6">
            <div className="flex flex-col items-center">
              <svg
                className="w-12 h-12 text-pierre-violet"
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
              <h1 className="mt-4 text-xl font-bold text-white tracking-tight">
                Reset Your Password
              </h1>
              <p className="mt-1 text-sm text-zinc-400 text-center">
                Enter your email and we&apos;ll send you a 6-digit code
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

              <Input
                id="email"
                name="email"
                type="email"
                label="Email address"
                autoComplete="email"
                required
                placeholder="Enter your email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                variant="dark"
              />

              <Button
                type="submit"
                variant="gradient"
                loading={isLoading}
                className="w-full shadow-glow-sm"
              >
                {isLoading ? 'Sending code...' : 'Send reset code'}
              </Button>

              <div className="text-center">
                <button
                  type="button"
                  onClick={onNavigateToLogin}
                  className="text-sm text-pierre-violet-light hover:text-pierre-cyan-light font-medium transition-colors"
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
