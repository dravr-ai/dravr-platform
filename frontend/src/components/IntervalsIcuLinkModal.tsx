// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Link modal for the Intervals.icu provider — collects athlete id + API key
// ABOUTME: Intervals.icu is API-key (HTTP Basic) not OAuth; the server validates the pair live before storing

import { useEffect, useId, useState } from 'react';
import { providersApi } from '../services/api';
import { useTranslation } from '@pierre/i18n';
import { describeApiError } from '@pierre/ui-logic';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
import { useDialog } from '../hooks/useDialog';
import { RevealButton } from './ui';

interface IntervalsIcuLinkModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConnected: () => void;
}

// AppError serialises as { code, message, ... }; some in-band errors expose
// { error }. Prefer message, then error, then the raw error as a last resort.
export default function IntervalsIcuLinkModal({
  isOpen,
  onClose,
  onConnected,
}: IntervalsIcuLinkModalProps) {
  const { t } = useTranslation();
  const online = useOnlineStatus();
  // This overlay renders its own chrome instead of composing <Modal>, so it
  // carried none of the dialog contract: no role, no Escape, no scroll lock,
  // and Tab walked straight out into the page behind an API-key form.
  const titleId = useId();
  const { containerRef } = useDialog({ open: isOpen, onClose });
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
      setError(describeApiError(err, { online, t, fallbackKey: 'shell.intervalsLinkFailed' }));
      setIsLoading(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
    >
      <div
        ref={containerRef}
        tabIndex={-1}
        className="bg-surface-container-highest rounded-2xl border ghost-border max-w-md w-full mx-4 overflow-hidden"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b ghost-border">
          <div className="flex items-center gap-3">
            {/* Intervals.icu brand colours, not Boreal tokens. This is the
                provider mark next to its name; recolouring it would
                misrepresent which service is being connected. */}
            <div className="w-8 h-8 rounded-lg bg-gradient-to-br from-sky-500 to-blue-600 flex items-center justify-center flex-shrink-0">
              <svg className="w-4 h-4 text-on-surface" viewBox="0 0 24 24" fill="currentColor">
                <path d="M3 13h4l3 7 4-14 3 7h4" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
              </svg>
            </div>
            <div>
              <h2 id={titleId} className="text-lg font-semibold text-on-surface">{t('shell.intervalsConnectTitle')}</h2>
              <p className="text-xs text-on-surface/50">{t('shell.intervalsApiKeyHint')}</p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="p-2 rounded-lg hover:bg-surface-container text-on-surface/60 hover:text-on-surface transition-colors"
            title={t('common.close')}
            aria-label={t('shell.intervalsCloseModalAria')}
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
                {t('frag.findAthleteIdUnder')}{' '}
                <span className="text-on-surface font-medium">{t('shell.intervalsSettingsPath')}</span> on
                intervals.icu.
              </p>

              <div>
                <label htmlFor="intervals-athlete-id" className="block text-sm text-on-surface/60 mb-1.5">
                  {t('shell.intervalsAthleteId')}
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
                  {t('shell.intervalsApiKeyLabel')}
                </label>
                <div className="relative">
                  <input
                    id="intervals-api-key"
                    type={showKey ? 'text' : 'password'}
                    placeholder={t('shell.intervalsApiKeyPlaceholder')}
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    className="input-glass w-full pr-10"
                    required
                    autoComplete="off"
                    name="api_key"
                  />
                  <RevealButton
  revealed={showKey}
  onToggle={() => setShowKey(!showKey)}
  label={showKey ? t('shell.intervalsHideApiKey') : t('shell.intervalsShowApiKey')}
/>
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
                className="w-full py-3 bg-primary rounded-lg text-on-primary font-medium /40 hover:-translate-y-0.5 transition-all disabled:opacity-50 disabled:cursor-not-allowed disabled:transform-none"
              >
                {isLoading ? t('shell.intervalsVerifying') : t('shell.intervalsConnectAction')}
              </button>
            </form>
          )}
        </div>
      </div>
    </div>
  );
}
