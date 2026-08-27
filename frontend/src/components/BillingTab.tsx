// ABOUTME: Phase 2 admin Billing tab — CSV/JSON export of monthly LLM usage rows
// ABOUTME: Calls /api/admin/billing/export and triggers a browser download
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { adminApi } from '../services/api';
import { Button, Card } from './ui';
import { useTranslation } from '@pierre/i18n';

function defaultPeriod(): string {
  const now = new Date();
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, '0');
  return `${y}-${m}`;
}

export default function BillingTab() {
  const { t } = useTranslation();
  const [period, setPeriod] = useState<string>(defaultPeriod());
  const [limit, setLimit] = useState<number>(1000);
  const [busy, setBusy] = useState<boolean>(false);
  const [error, setError] = useState<string | null>(null);

  async function handleExport(format: 'csv' | 'json') {
    setError(null);
    setBusy(true);
    try {
      const data = await adminApi.exportBillingCsv(period, format, limit);
      const blob =
        format === 'csv'
          ? (data as Blob)
          : new Blob([JSON.stringify(data, null, 2)], {
              type: 'application/json',
            });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `billing-${period}.${format}`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setError(e instanceof Error ? e.message : t('shell.billingExportFailed'));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6">
      <Card variant="dark" className="p-6">
        <h2 className="text-lg font-semibold text-on-surface mb-2">
          {t('shell.billingExportTitle')}
        </h2>
        <p className="text-sm text-on-surface-variant mb-4">
          {t('shell.billingExportHint')}
        </p>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-4">
          <div>
            <label
              htmlFor="billing-period"
              className="block text-sm font-medium text-on-surface mb-1"
            >
              {t('shell.billingExportPeriod')}
            </label>
            <input
              id="billing-period"
              type="text"
              value={period}
              onChange={(e) => setPeriod(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border ghost-border bg-surface-container text-on-surface"
              placeholder="2026-04"
            />
          </div>
          <div>
            <label
              htmlFor="billing-limit"
              className="block text-sm font-medium text-on-surface mb-1"
            >
              {t('shell.billingExportRowLimit')}
            </label>
            <input
              id="billing-limit"
              type="number"
              min={1}
              max={10000}
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
              className="w-full px-3 py-2 rounded-lg border ghost-border bg-surface-container text-on-surface"
            />
          </div>
        </div>

        <div className="flex items-center space-x-3">
          <Button
            onClick={() => handleExport('csv')}
            disabled={busy}
            className="bg-activity hover:bg-activity/80 text-on-primary"
          >
            {busy ? t('shell.billingExporting') : t('shell.billingExportCsv')}
          </Button>
          <Button
            onClick={() => handleExport('json')}
            disabled={busy}
            variant="secondary"
          >
            {t('shell.billingExportJson')}
          </Button>
        </div>

        {error && (
          <p className="mt-4 text-sm text-error">{t('frag.errorLabel')} {error}</p>
        )}
      </Card>
    </div>
  );
}
