// ABOUTME: Privacy settings tab for managing analytics consent
// ABOUTME: Allows users to opt in/out of anonymized usage analytics tracking
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState, useEffect } from 'react';
import { useMutation } from '@tanstack/react-query';
import { userApi } from '../services/api';
import { Card } from './ui';
import { useAuth } from '../hooks/useAuth';

export default function PrivacySettingsTab() {
  const { user } = useAuth();
  const [enabled, setEnabled] = useState(false);
  const [message, setMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null);

  useEffect(() => {
    if (user?.analytics_consent != null) {
      setEnabled(user.analytics_consent);
    }
  }, [user?.analytics_consent]);

  const mutation = useMutation({
    mutationFn: (consent: boolean) => userApi.updateAnalyticsConsent(consent),
    onSuccess: (data) => {
      setMessage({ type: 'success', text: data.message });
      setTimeout(() => setMessage(null), 3000);
    },
    onError: () => {
      setEnabled(!enabled);
      setMessage({ type: 'error', text: 'Failed to update analytics consent' });
      setTimeout(() => setMessage(null), 3000);
    },
  });

  const handleToggle = () => {
    const newValue = !enabled;
    setEnabled(newValue);
    mutation.mutate(newValue);
  };

  return (
    <Card variant="dark">
      <h2 className="text-lg font-semibold text-on-surface mb-6">Privacy & Data</h2>

      <div className="space-y-6">
        {/* Analytics Consent Toggle */}
        <div className="flex items-start justify-between gap-4 p-4 bg-surface-container-low rounded-xl border ghost-border">
          <div className="flex-1">
            <div className="flex items-center gap-3 mb-2">
              <div className="w-10 h-10 rounded-xl bg-pierre-violet/15 flex items-center justify-center flex-shrink-0">
                <svg className="w-5 h-5 text-pierre-violet" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M9 19v-6a2 2 0 00-2-2H5a2 2 0 00-2 2v6a2 2 0 002 2h2a2 2 0 002-2zm0 0V9a2 2 0 012-2h2a2 2 0 012 2v10m-6 0a2 2 0 002 2h2a2 2 0 002-2m0 0V5a2 2 0 012-2h2a2 2 0 012 2v14a2 2 0 01-2 2h-2a2 2 0 01-2-2z" />
                </svg>
              </div>
              <h3 className="text-sm font-medium text-on-surface">Usage Analytics</h3>
            </div>
            <p className="text-sm text-on-surface-variant leading-relaxed">
              Help improve Pierre by sharing anonymized usage data. We track general usage
              patterns like which features you use and how often — never your personal data,
              messages, or fitness information. All identifiers are cryptographically hashed
              before leaving your device.
            </p>
          </div>

          {/* Toggle Switch */}
          <button
            type="button"
            role="switch"
            aria-checked={enabled}
            onClick={handleToggle}
            disabled={mutation.isPending}
            className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-pierre-violet focus:ring-offset-2 focus:ring-offset-zinc-900 disabled:opacity-50 ${
              enabled ? 'bg-pierre-violet' : 'bg-zinc-600'
            }`}
          >
            <span
              className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out ${
                enabled ? 'translate-x-5' : 'translate-x-0'
              }`}
            />
          </button>
        </div>

        {/* What We Track */}
        <div className="p-4 bg-surface-container-low rounded-xl border ghost-border">
          <h3 className="text-sm font-medium text-on-surface mb-3">What we collect when enabled</h3>
          <ul className="space-y-2 text-sm text-on-surface-variant">
            <li className="flex items-start gap-2">
              <svg className="w-4 h-4 text-emerald-400 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
              Feature usage patterns (which tools and commands you use)
            </li>
            <li className="flex items-start gap-2">
              <svg className="w-4 h-4 text-emerald-400 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
              Session frequency and messaging channel usage
            </li>
            <li className="flex items-start gap-2">
              <svg className="w-4 h-4 text-emerald-400 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
              </svg>
              Error rates and performance metrics
            </li>
          </ul>

          <h3 className="text-sm font-medium text-on-surface mt-4 mb-3">What we never collect</h3>
          <ul className="space-y-2 text-sm text-on-surface-variant">
            <li className="flex items-start gap-2">
              <svg className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
              Your messages, conversations, or fitness data
            </li>
            <li className="flex items-start gap-2">
              <svg className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
              Personal information (email, name, location)
            </li>
            <li className="flex items-start gap-2">
              <svg className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
              </svg>
              Your real user ID (all identifiers are SHA-256 hashed)
            </li>
          </ul>
        </div>

        {/* Status Message */}
        {message && (
          <div
            className={`p-3 rounded-lg text-sm ${
              message.type === 'success'
                ? 'bg-emerald-500/10 text-emerald-400 border border-emerald-500/20'
                : 'bg-red-500/10 text-red-400 border border-red-500/20'
            }`}
          >
            {message.text}
          </div>
        )}
      </div>
    </Card>
  );
}
