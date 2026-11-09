// ABOUTME: The file step of an import whose export is a file the athlete holds: pick it, send it, follow the bytes, resume or cancel
// ABOUTME: The bytes go in chunks straight to the store's session, read a slice at a time, so a multi-gigabyte file is never held in memory
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useRef, useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { uploadExport, type UploadBytes } from '@pierre/api-client';
import type { ArchiveImport } from '@pierre/shared-types';
import { uploadErrorKey, uploadPercent, type UploadSourceCopy } from '@pierre/ui-logic';
import { importsApi } from '../../services/api';
import { Button, FilePickerButton } from '../ui';

interface UploadIntakeProps {
  copy: UploadSourceCopy;
  /** The source's latest import: one still waiting is resumed rather than opened anew. */
  current: ArchiveImport | null | undefined;
  /** The import changed: read it again. */
  onChanged: () => void;
}

/** A browser file, read a slice at a time. */
function bytesOf(file: File): UploadBytes {
  return { size: file.size, read: async (start, end) => file.slice(start, end) };
}

export default function UploadIntake({ copy, current, onChanged }: UploadIntakeProps) {
  const { t } = useTranslation();
  const { source } = copy;
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const stop = useRef<AbortController | null>(null);
  const waiting = current?.status === 'awaiting_link' ? current : null;

  useEffect(() => () => stop.current?.abort(), []);

  const upload = useMutation({
    mutationFn: (file: File) => {
      const controller = new AbortController();
      stop.current = controller;
      return uploadExport({
        api: importsApi,
        source,
        file: bytesOf(file),
        waitingImportId: waiting?.id ?? null,
        onOpened: onChanged,
        onProgress: (committed, total) => setPercent(uploadPercent(committed, total)),
        signal: controller.signal,
      });
    },
    onMutate: () => {
      setError(null);
      setPercent(0);
    },
    onSuccess: onChanged,
    onError: (err) => {
      if (!stop.current?.signal.aborted) setError(t(uploadErrorKey(err, copy)));
      onChanged();
    },
    onSettled: () => setPercent(null),
  });

  const cancel = useMutation({
    mutationFn: (importId: string) => {
      stop.current?.abort();
      return importsApi.cancelImport(source, importId);
    },
    onSuccess: onChanged,
    onError: () => setError(t('providers.import.strava.genericError')),
  });

  return (
    <section className="border-t ghost-border-faint py-4" data-testid="import-upload">
      <h3 className="text-sm font-semibold text-on-surface">{t(copy.step2Title)}</h3>
      {percent === null && (
        <p className="mt-1 text-sm text-on-surface-variant" data-testid="import-upload-hint">
          {t(waiting ? copy.resumeHint : copy.pickHint)}
        </p>
      )}
      {percent !== null ? (
        <div className="mt-3">
          <p className="text-sm text-on-surface" role="status" data-testid="import-upload-progress">
            {t(copy.uploading, { percent })}
          </p>
          <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-surface-container-high" aria-hidden="true">
            <div className="h-full bg-primary transition-all" data-testid="import-upload-bar" style={{ width: `${percent}%` }} />
          </div>
          <p className="mt-2 text-xs text-on-surface-variant">{t(copy.keepOpen)}</p>
        </div>
      ) : (
        <div className="mt-3">
          <FilePickerButton
            accept={copy.accept}
            onPick={(file) => upload.mutate(file)}
            className="rounded text-sm font-medium text-primary hover:underline focus-ring touch-target disabled:opacity-60 disabled:no-underline"
            data-testid="import-upload-pick"
          >
            {t(waiting ? copy.resume : copy.pick)}
          </FilePickerButton>
        </div>
      )}
      {waiting && (
        <Button
          className="mt-3"
          variant="secondary"
          size="sm"
          onClick={() => cancel.mutate(waiting.id)}
          loading={cancel.isPending}
          data-testid="import-upload-cancel"
        >
          {t('providers.import.strava.cancel')}
        </Button>
      )}
      {error && (
        <p role="alert" className="mt-2 text-sm text-error" data-testid="import-upload-error">
          {error}
        </p>
      )}
    </section>
  );
}
