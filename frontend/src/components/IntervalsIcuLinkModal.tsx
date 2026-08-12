// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Link modal for the Intervals.icu provider — collects athlete id + API key
// ABOUTME: Intervals.icu is API-key (HTTP Basic) not OAuth; the server validates the pair live before storing

import { useEffect, useState } from 'react';
import { providersApi } from '../services/api';

interface IntervalsIcuLinkModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConnected: () => void;
}

// AppError serialises as { code, message, ... }; some in-band errors expose
// { error }. Prefer message, then error, then the raw error as a last resort.
function extractErrorMessage(err: unknown, prefix: string): string {
  const data = (err as { response?: { data?: { message?: string; error?: string } } })?.response
    ?.data;
  const detail = data?.message || data?.error;
  if (detail) return detail;
  return `${prefix}: ${err}`;
}

export default function IntervalsIcuLinkModal({
  isOpen,
  onClose,
  onConnected,
}: IntervalsIcuLinkModalProps) {
  const [athleteId, setAthleteId] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  // Reset transient state whenever the modal reopens.
  useEffect(() => {
    if (isOpen) {
      setAthleteId('');
      setApiKey('');
      setShowKey(false);
      setIsLoading(false);
      setError(null);
      setSuccess(null);
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsLoading(true);
    setError(null);
    try {
      const result = await providersApi.linkIntervalsIcu({
        athlete_id: athleteId.trim(),
        api_key: apiKey.trim(),
      });
      const name = result.athlete?.name || result.athlete?.id || 'your account';
      setSuccess(`Connected ${name}`);
      // Brief confirmation, then hand control back to the parent (which refetches).
      setTimeout(onConnected, 1200);
    } catch (err) {
      setError(extractErrorMessage(err, 'Could not link Intervals.icu'));
      setIsLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm">
      <div className="bg-surface-container-highest rounded-2xl border ghost-border shadow-2xl max-w-md w-full mx-4 overflow-hidden">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b ghost-border">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-sky-500 to-blue-600 flex items-center justify-center flex-shrink-0">
              <svg className="w-4 h-4 text-on-surface" viewBox="0 0 24 24" fill="currentColor">
                <path d="M3 13h4l3 7 4-14 3 7h4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </div>
            <div>
              <h2 className="text-lg font-semibold text-on-surface">Connect Intervals.icu</h2>
              <p className="text-xs text-on-surface/50">API key — no OAuth redirect</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 rounded-lg hover:bg-surface-container text-on-surface/60 hover:text-on-surface transition-colors"
            title="Close"
            aria-label="Close Intervals.icu link modal"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <div className="p-6">
          {success ? (
            <div className="flex flex-col items-center gap-3 py-4 text-center">
              <div className="w-12 h-12 rounded-full bg-success/15 flex items-center justify-center">
                <svg className="w-6 h-6 text-success" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 13l4 4L19 7" />
                </svg>
              </div>
              <p className="text-on-surface font-medium">{success}</p>
            </div>
          ) : (
            <form onSubmit={handleSubmit} className="space-y-4">
              <p className="text-sm text-on-surface-variant leading-snug">
                Find your athlete id and API key under{' '}
                <span className="text-on-surface font-medium">Settings → Developer</span> on
                intervals.icu.
              </p>

              <div>
                <label htmlFor="intervals-athlete-id" className="block text-sm text-on-surface/60 mb-1.5">
                  Athlete ID
                </label>
                <input
                  id="intervals-athlete-id"
                  type="text"
                  placeholder="i123456"
                  value={athleteId}
                  onChange={(e) => setAthleteId(e.target.value)}
                  className="input-glass w-full"
                  required
                  autoFocus
                  autoComplete="off"
                  name="athlete_id"
                />
              </div>

              <div>
                <label htmlFor="intervals-api-key" className="block text-sm text-on-surface/60 mb-1.5">
                  API Key
                </label>
                <div className="relative">
                  <input
                    id="intervals-api-key"
                    type={showKey ? 'text' : 'password'}
                    placeholder="API key"
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    className="input-glass w-full pr-10"
                    required
                    autoComplete="off"
                    name="api_key"
                  />
                  <button
                    type="button"
                    onClick={() => setShowKey(!showKey)}
                    className="absolute right-3 top-1/2 -translate-y-1/2 text-on-surface/40 hover:text-on-surface/70 transition-colors"
                    title={showKey ? 'Hide API key' : 'Show API key'}
                    aria-label={showKey ? 'Hide API key' : 'Show API key'}
                  >
                    {showKey ? (
                      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13.875 18.825A10.05 10.05 0 0112 19c-4.478 0-8.268-2.943-9.543-7a9.97 9.97 0 011.563-3.029m5.858.908a3 3 0 114.243 4.243M9.878 9.878l4.242 4.242M9.88 9.88l-3.29-3.29m7.532 7.532l3.29 3.29M3 3l3.59 3.59m0 0A9.953 9.953 0 0112 5c4.478 0 8.268 2.943 9.543 7a10.025 10.025 0 01-4.132 5.411m0 0L21 21" />
                      </svg>
                    ) : (
                      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
                      </svg>
                    )}
                  </button>
                </div>
              </div>

              {error && (
                <p className="text-sm text-error bg-error/10 border border-error/20 rounded-lg px-3 py-2">
                  {error}
                </p>
              )}

              <button
                type="submit"
                disabled={isLoading || !athleteId.trim() || !apiKey.trim()}
                className="w-full py-3 bg-gradient-to-r from-sky-500 to-blue-600 rounded-lg text-on-surface font-medium hover:shadow-lg hover:shadow-sky-500/40 hover:-translate-y-0.5 transition-all disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none"
              >
                {isLoading ? 'Verifying…' : 'Connect'}
              </button>
            </form>
          )}
        </div>
      </div>
    </div>
  );
}
