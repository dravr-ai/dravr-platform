// ABOUTME: Provider-agnostic BYO OAuth app modal — captures client_id/secret/redirect URI and stores them
// ABOUTME: Used inline by onboarding flows so first-run users never need to navigate to Settings

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { userApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button, Input } from './ui';
import { useTranslation } from '@pierre/i18n';

interface OAuthAppSetupModalProps {
  isOpen: boolean;
  onClose: () => void;
  /** Fires after the OAuth app is saved successfully. */
  onSaved: () => void;
  /** Provider id (matches the server-side provider enum, e.g. `whoop`, `strava`). */
  provider: string;
  /** Human-readable name shown in the modal title and copy. */
  displayName: string;
  /** Developer-portal URL where the user creates their OAuth app. */
  devPortalUrl: string;
}

export default function OAuthAppSetupModal({
  isOpen,
  onClose,
  onSaved,
  provider,
  displayName,
  devPortalUrl,
}: OAuthAppSetupModalProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const defaultRedirectUri = `${window.location.origin}/api/oauth/callback/${provider}`;
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [redirectUri, setRedirectUri] = useState(defaultRedirectUri);
  const [error, setError] = useState<string | null>(null);

  // Pre-populate if the user already saved this provider's app earlier (e.g.
  // they configured it in Settings and are now back here).
  const { data: appsResponse } = useQuery({
    queryKey: QUERY_KEYS.user.oauthApps(),
    queryFn: () => userApi.getOAuthApps(),
    enabled: isOpen,
  });

  useEffect(() => {
    if (!isOpen) return;
    setError(null);
    const existing = appsResponse?.apps?.find((a) => a.provider === provider);
    if (existing) {
      setClientId(existing.client_id ?? '');
      // Client secret is never returned by the API; user must re-enter it.
      setClientSecret('');
      setRedirectUri(existing.redirect_uri ?? defaultRedirectUri);
    } else {
      setClientId('');
      setClientSecret('');
      setRedirectUri(defaultRedirectUri);
    }
  }, [isOpen, appsResponse, provider, defaultRedirectUri]);

  const saveMutation = useMutation({
    mutationFn: (data: { client_id: string; client_secret: string; redirect_uri: string }) =>
      userApi.registerOAuthApp({ provider, ...data }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.oauthApps() });
      onSaved();
    },
    onError: (err: unknown) => {
      const message =
        (err as { response?: { data?: { message?: string } } })?.response?.data?.message ??
        t('app.failedSaveOauthApp', { provider: displayName });
      setError(message);
    },
  });

  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!clientId.trim() || !clientSecret.trim() || !redirectUri.trim()) {
      setError(t('app.oauthAllFieldsRequired'));
      return;
    }
    saveMutation.mutate({
      client_id: clientId.trim(),
      client_secret: clientSecret.trim(),
      redirect_uri: redirectUri.trim(),
    });
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 px-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="oauth-setup-title"
    >
      <div className="w-full max-w-md rounded-2xl bg-surface-container border border-outline-variant/30 shadow-xl">
        <div className="px-6 py-5 border-b border-outline-variant/30 flex items-center justify-between">
          <h2 id="oauth-setup-title" className="text-lg font-semibold text-on-surface">
            {t('app.setUpOauthApp', { provider: displayName })}
          </h2>
          <button
            type="button"
            onClick={onClose}
            className="text-on-surface-variant hover:text-on-surface"
            aria-label={t('common.close')}
          >
            <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}>
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        <form onSubmit={handleSubmit} className="px-6 py-5 space-y-4">
          <p className="text-sm text-on-surface-variant">
            {displayName} requires each user to register their own developer app. Create one at{' '}
            <a
              href={devPortalUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-primary underline"
            >
              {new URL(devPortalUrl).host}
            </a>{' '}
            and use the redirect URI shown below.
          </p>

          <div>
            <label
              htmlFor="oauth-client-id"
              className="block text-xs font-semibold text-on-surface-variant mb-1.5"
            >
              CLIENT ID
            </label>
            <Input
              id="oauth-client-id"
              value={clientId}
              onChange={(e) => setClientId(e.target.value)}
              placeholder={t('app.pasteProviderClientIdLower', { provider: displayName })}
              autoComplete="off"
              required
            />
          </div>

          <div>
            <label
              htmlFor="oauth-client-secret"
              className="block text-xs font-semibold text-on-surface-variant mb-1.5"
            >
              CLIENT SECRET
            </label>
            <Input
              id="oauth-client-secret"
              type="password"
              value={clientSecret}
              onChange={(e) => setClientSecret(e.target.value)}
              placeholder={t('app.pasteProviderClientSecretLower', { provider: displayName })}
              autoComplete="off"
              required
            />
          </div>

          <div>
            <label
              htmlFor="oauth-redirect-uri"
              className="block text-xs font-semibold text-on-surface-variant mb-1.5"
            >
              REDIRECT URI
            </label>
            <Input
              id="oauth-redirect-uri"
              value={redirectUri}
              onChange={(e) => setRedirectUri(e.target.value)}
              autoComplete="off"
              required
            />
            <p className="text-xs text-on-surface-variant/70 mt-1.5">
              {t('app.addExactUri', { provider: displayName })}
            </p>
          </div>

          {error && (
            <div
              role="alert"
              className="rounded-lg border border-error/40 bg-error/10 px-3 py-2 text-sm text-error"
            >
              {error}
            </div>
          )}

          <div className="flex gap-3 pt-2">
            <Button type="button" variant="secondary" onClick={onClose} className="flex-1">
              {t('common.cancel')}
            </Button>
            <Button type="submit" disabled={saveMutation.isPending} className="flex-1">
              {saveMutation.isPending ? t('app.saving') : t('app.saveAndConnect')}
            </Button>
          </div>
        </form>
      </div>
    </div>
  );
}
