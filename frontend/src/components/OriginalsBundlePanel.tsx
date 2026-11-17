// ABOUTME: The originals bundle on the web privacy pane — ask for every kept original file as one ZIP, optionally sealed
// ABOUTME: Follows the build until it is ready, then saves it from its short signed URL or as the file the server streams

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useState } from 'react';
import { useMutation, useQuery } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  apiErrorStatus,
  exportRefusalCode,
  ORIGINALS_POLL_MS,
  originalsIncomplete,
  originalsRefusalKey,
  originalsStateKey,
  originalsStillBuilding,
  sealPassphraseAccepted,
} from '@pierre/ui-logic';
import { dataExportApi, originalsApi, type UserDataExportFile } from '../services/api';
import type { OriginalsBundle } from '@pierre/api-client';
import { Button, Input } from './ui';

/** Hand `file` to the browser as a download. */
function saveFile({ fileName, content }: UserDataExportFile): void {
  const url = URL.createObjectURL(content);
  const link = document.createElement('a');
  link.href = url;
  link.download = fileName;
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(url);
}

/** Send the browser to a signed URL; the storage names the file as an attachment. */
function openSigned(url: string): void {
  const link = document.createElement('a');
  link.href = url;
  link.rel = 'noopener noreferrer';
  document.body.appendChild(link);
  link.click();
  link.remove();
}

export default function OriginalsBundlePanel() {
  const { t, i18n } = useTranslation();
  const [passphrase, setPassphrase] = useState('');
  const [asked, setAsked] = useState<OriginalsBundle | null>(null);
  const [failure, setFailure] = useState<string | null>(null);

  // The latest bundle, so a page left while it built finds it again.
  const latest = useQuery({
    queryKey: ['originals-latest'],
    queryFn: () => originalsApi.getLatestOriginals(),
  });
  const current = asked ?? latest.data ?? null;
  const followed = useQuery({
    queryKey: ['originals-bundle', current?.bundleId],
    queryFn: () => originalsApi.getOriginals(current!.bundleId),
    enabled: originalsStillBuilding(current),
    refetchInterval: (query) =>
      originalsStillBuilding(query.state.data ?? current) ? ORIGINALS_POLL_MS : false,
  });
  const bundle = followed.data ?? current;

  const request = useMutation({
    mutationFn: () => originalsApi.requestOriginals(passphrase === '' ? undefined : passphrase),
    onMutate: () => setFailure(null),
    onSuccess: (started) => {
      setPassphrase('');
      setAsked(started);
    },
    onError: (err: unknown) => setFailure(t(originalsRefusalKey(apiErrorStatus(err), exportRefusalCode(err)))),
  });

  const download = useMutation({
    mutationFn: (ready: OriginalsBundle) => dataExportApi.downloadOriginals(ready),
    onMutate: () => setFailure(null),
    onSuccess: (target) => (target.kind === 'url' ? openSigned(target.url) : saveFile(target.file)),
    onError: () => setFailure(t('dataExport.originalsFailed')),
  });

  const sealOk = sealPassphraseAccepted(passphrase);
  const building = originalsStillBuilding(bundle);
  const expires = bundle ? new Date(bundle.expiresAt).toLocaleDateString(i18n.language) : '';

  return (
    <div className="mt-6 border-t border-outline-variant pt-4" data-testid="originals-bundle-panel">
      <h3 className="text-sm font-medium text-on-surface">{t('dataExport.originalsTitle')}</h3>
      <p className="mt-1 text-sm text-on-surface-variant">{t('dataExport.originalsDescription')}</p>
      <div className="mt-3 max-w-md">
        <Input
          type="password"
          autoComplete="new-password"
          label={t('dataExport.originalsSealLabel')}
          helpText={t('dataExport.originalsSealHint')}
          error={sealOk ? undefined : t('dataExport.originalsSealTooShort')}
          value={passphrase}
          onChange={(event) => setPassphrase(event.target.value)}
          disabled={building || request.isPending}
          data-testid="originals-passphrase"
        />
      </div>
      <Button
        className="mt-3"
        variant="secondary"
        size="sm"
        onClick={() => request.mutate()}
        loading={request.isPending}
        disabled={!sealOk || building}
        data-testid="originals-request"
      >
        {t('dataExport.originalsAction')}
      </Button>
      {bundle && (
        <div className="mt-3 space-y-2" role="status" data-testid="originals-state">
          <p className="text-sm text-on-surface">
            {t(originalsStateKey(bundle), { files: bundle.files ?? 0, date: expires })}
          </p>
          {bundle.sealed && bundle.status === 'ready' && (
            <p className="text-sm text-on-surface-variant">{t('dataExport.originalsSealed')}</p>
          )}
          {originalsIncomplete(bundle) && (
            <p className="text-sm text-on-surface-variant" data-testid="originals-incomplete">
              {t('dataExport.originalsIncomplete')}
            </p>
          )}
          {bundle.status === 'ready' && (
            <Button
              variant="primary"
              size="sm"
              onClick={() => download.mutate(bundle)}
              loading={download.isPending}
              data-testid="originals-download"
            >
              {t('dataExport.originalsDownload')}
            </Button>
          )}
        </div>
      )}
      {failure && (
        <p className="mt-3 text-sm text-error" role="alert" data-testid="originals-failure">
          {failure}
        </p>
      )}
    </div>
  );
}
