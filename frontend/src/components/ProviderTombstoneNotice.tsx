// ABOUTME: Why a provider account cannot be connected — it belonged to a deleted Dravr account — and the start-fresh action
// ABOUTME: Clears the tombstone the refusal named, then hands back to the caller to run that provider's connect again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useTranslation } from '@pierre/i18n';
import { describeApiError, describeProviderTombstone, type ImportTombstoneRefusal } from '@pierre/ui-logic';
import { userApi } from '../services/api';
import { useOnlineStatus } from '../hooks/useOnlineStatus';
import { Button } from './ui';

interface ProviderTombstoneNoticeProps {
  refusal: ImportTombstoneRefusal;
  /** The brand the provider's card shows. */
  providerName: string;
  /** Runs once the tombstone is cleared: the provider's connect, again. */
  onCleared: () => void;
}

/**
 * The refusal of a provider account a deleted account held, worded with its
 * deletion date, and the one action that reopens it. Nothing reopens on its
 * own: the athlete chooses to start fresh with the account's data deleted.
 */
export default function ProviderTombstoneNotice({ refusal, providerName, onCleared }: ProviderTombstoneNoticeProps) {
  const { t, language } = useTranslation();
  const online = useOnlineStatus();
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleClear = async () => {
    setClearing(true);
    setError(null);
    try {
      await userApi.clearImportTombstone(refusal.tombstoneId);
      onCleared();
    } catch (err) {
      setError(describeApiError(err, { online, t, fallbackKey: 'errors.unknown' }));
      setClearing(false);
    }
  };

  return (
    <div className="flex flex-col gap-4" data-testid="provider-tombstone-notice">
      <p className="text-sm text-on-surface-variant">
        {describeProviderTombstone(refusal, providerName, { language, t })}
      </p>
      <Button
        variant="primary"
        size="lg"
        className="w-full"
        loading={clearing}
        onClick={() => void handleClear()}
        data-testid="provider-tombstone-clear"
      >
        {t('providers.connectTombstonedClear')}
      </Button>
      {error && (
        <p className="text-xs text-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
