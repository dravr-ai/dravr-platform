// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › API keys — the athlete's own keys: create one (its secret shown once), see each key's use, revoke it
// ABOUTME: Server state through React Query over @pierre/api-client's apiKeys domain; revoked keys leave the list

import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { ApiKeyCreateResponse, ApiKeyInfo } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { formatDateTime } from '@pierre/chat-utils';
import { describeApiError } from '@pierre/ui-logic';
import { API_KEY_USAGE_WINDOW_DAYS } from '@pierre/shared-constants';
import { apiKeysApi } from '../../services/api';
import { Button, ConfirmDialog, Input, Section } from '../ui';

const API_KEYS_KEY = ['api-keys'] as const;
const usageKey = (keyId: string) => [...API_KEYS_KEY, 'usage', keyId] as const;
const DAY_MS = 24 * 60 * 60 * 1000;

/** The "N requests in the last 30 days" line under one key. */
function KeyUsage({ keyId }: { keyId: string }) {
  const { t } = useTranslation();
  const usage = useQuery({
    queryKey: usageKey(keyId),
    queryFn: () => {
      const end = new Date();
      const start = new Date(end.getTime() - API_KEY_USAGE_WINDOW_DAYS * DAY_MS);
      return apiKeysApi.usage(keyId, start.toISOString(), end.toISOString());
    },
  });
  if (!usage.data) return null;
  return (
    <span data-testid={`api-key-usage-${keyId}`}>
      {t('apiKeys.requests30d', { count: usage.data.stats.total_requests })}
    </span>
  );
}

function KeyRow({ apiKey, onRevoke }: { apiKey: ApiKeyInfo; onRevoke: (key: ApiKeyInfo) => void }) {
  const { t, i18n } = useTranslation();
  const date = (iso: string) => formatDateTime(iso, i18n.language);
  return (
    <li className="flex items-start justify-between gap-3 py-3" data-testid={`api-key-${apiKey.id}`}>
      <div className="min-w-0">
        <p className="text-sm font-medium text-on-surface break-words">{apiKey.name}</p>
        <p className="text-xs text-on-surface-variant font-mono">{apiKey.key_prefix}…</p>
        <p className="mt-0.5 flex flex-wrap gap-x-3 text-xs text-on-surface-variant">
          <span>{t('apiKeys.createdOn', { date: date(apiKey.created_at) })}</span>
          <span>
            {apiKey.last_used_at
              ? t('apiKeys.lastUsed', { date: date(apiKey.last_used_at) })
              : t('apiKeys.neverUsed')}
          </span>
          {apiKey.expires_at && <span>{t('apiKeys.expiresOn', { date: date(apiKey.expires_at) })}</span>}
          <KeyUsage keyId={apiKey.id} />
        </p>
      </div>
      <Button
        type="button"
        variant="secondary"
        size="sm"
        onClick={() => onRevoke(apiKey)}
        data-testid={`api-key-revoke-${apiKey.id}`}
      >
        {t('apiKeys.revoke')}
      </Button>
    </li>
  );
}

/** The new key, shown this once, with a way to copy it. */
function CreatedKey({ created, onDone }: { created: ApiKeyCreateResponse; onDone: () => void }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(created.api_key);
    setCopied(true);
  };
  return (
    <div className="rounded-lg border ghost-border bg-surface-container p-4" role="status" data-testid="api-key-created">
      <p className="text-sm font-semibold text-on-surface">{t('apiKeys.createdTitle')}</p>
      <p className="mt-0.5 text-sm text-on-surface-variant">{t('apiKeys.createdHint')}</p>
      <code
        className="mt-3 block break-all rounded bg-surface px-3 py-2 font-mono text-sm text-on-surface"
        data-testid="api-key-secret"
      >
        {created.api_key}
      </code>
      <div className="mt-3 flex gap-2">
        <Button type="button" size="sm" onClick={copy} data-testid="api-key-copy">
          {copied ? t('apiKeys.copied') : t('apiKeys.copy')}
        </Button>
        <Button type="button" variant="secondary" size="sm" onClick={onDone} data-testid="api-key-done">
          {t('apiKeys.done')}
        </Button>
      </div>
    </div>
  );
}

export default function ApiKeysSettings() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [name, setName] = useState('');
  const [expiresInDays, setExpiresInDays] = useState('');
  const [created, setCreated] = useState<ApiKeyCreateResponse | null>(null);
  const [revoking, setRevoking] = useState<ApiKeyInfo | null>(null);

  const keys = useQuery({ queryKey: API_KEYS_KEY, queryFn: () => apiKeysApi.list() });

  const create = useMutation({
    mutationFn: () => {
      const days = Number.parseInt(expiresInDays, 10);
      return apiKeysApi.create({
        name: name.trim(),
        ...(Number.isFinite(days) && days > 0 ? { expires_in_days: days } : {}),
      });
    },
    onSuccess: (response) => {
      setCreated(response);
      setName('');
      setExpiresInDays('');
      void queryClient.invalidateQueries({ queryKey: API_KEYS_KEY });
    },
  });

  const revoke = useMutation({
    mutationFn: (keyId: string) => apiKeysApi.revoke(keyId),
    onSuccess: () => {
      setRevoking(null);
      void queryClient.invalidateQueries({ queryKey: API_KEYS_KEY });
    },
    onError: () => setRevoking(null),
  });

  const activeKeys = (keys.data?.api_keys ?? []).filter((key) => key.is_active);

  const renderList = () => {
    if (keys.isPending) {
      return <div className="pierre-spinner h-5 w-5" role="status" aria-label={t('common.loading')} />;
    }
    if (keys.isError) {
      return (
        <p role="alert" className="text-sm text-error">
          {describeApiError(keys.error, { t, fallbackKey: 'apiKeys.loadFailed' })}
        </p>
      );
    }
    if (activeKeys.length === 0) {
      return (
        <p className="text-sm text-on-surface-variant" data-testid="api-keys-empty">
          {t('apiKeys.empty')}
        </p>
      );
    }
    return (
      <ul className="divide-y divide-outline-variant" data-testid="api-keys-list">
        {activeKeys.map((key) => (
          <KeyRow key={key.id} apiKey={key} onRevoke={setRevoking} />
        ))}
      </ul>
    );
  };

  return (
    <div className="space-y-10" data-testid="api-keys-settings">
      <Section title={t('settingsTabs.apiKeys')} description={t('apiKeys.blurb')}>
        {created ? (
          <CreatedKey created={created} onDone={() => setCreated(null)} />
        ) : (
          <form
            className="grid gap-3 sm:grid-cols-[1fr_12rem_auto] sm:items-end"
            onSubmit={(event) => {
              event.preventDefault();
              if (name.trim()) create.mutate();
            }}
          >
            <Input
              label={t('apiKeys.nameLabel')}
              placeholder={t('apiKeys.namePlaceholder')}
              value={name}
              onChange={(event) => setName(event.target.value)}
              data-testid="api-key-name"
            />
            <Input
              label={t('apiKeys.expiresLabel')}
              type="number"
              min={1}
              value={expiresInDays}
              onChange={(event) => setExpiresInDays(event.target.value)}
              data-testid="api-key-expires"
            />
            <Button type="submit" disabled={!name.trim() || create.isPending} data-testid="api-key-create">
              {create.isPending ? t('apiKeys.creating') : t('apiKeys.create')}
            </Button>
            {/* Under the whole row, so the two fields keep one baseline. */}
            <p className="text-xs text-outline sm:col-span-3">{t('apiKeys.expiresHint')}</p>
          </form>
        )}
        {create.isError && (
          <p role="alert" className="mt-2 text-sm text-error">
            {describeApiError(create.error, { t, fallbackKey: 'apiKeys.createFailed' })}
          </p>
        )}
        <div className="mt-6">{renderList()}</div>
        {revoke.isError && (
          <p role="alert" className="mt-2 text-sm text-error">
            {describeApiError(revoke.error, { t, fallbackKey: 'apiKeys.revokeFailed' })}
          </p>
        )}
      </Section>
      <ConfirmDialog
        isOpen={revoking !== null}
        onClose={() => setRevoking(null)}
        onConfirm={() => revoking && revoke.mutate(revoking.id)}
        title={t('apiKeys.revokeTitle', { name: revoking?.name ?? '' })}
        message={t('apiKeys.revokeMessage')}
        confirmLabel={t('apiKeys.revoke')}
        cancelLabel={t('common.cancel')}
        variant="danger"
        isLoading={revoke.isPending}
      />
    </div>
  );
}
