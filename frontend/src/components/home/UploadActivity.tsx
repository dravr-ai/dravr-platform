// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The recent-activities section's upload of a completed workout's .fit file — the action, and the line saying how it went
// ABOUTME: The file goes up as the athlete picked it; a refusal is said by its reason, never by the server's prose

import { useTranslation } from '@pierre/i18n';
import { describeActivityUpload, type UseActivityUploadResult } from '@pierre/ui-logic';
import { FilePickerButton } from '../ui/FilePickerButton';

/** The upload action beside the section's title: a link-weight button over the system file picker. */
export function UploadActivityAction({ uploader }: { uploader: UseActivityUploadResult }) {
  const { t } = useTranslation();
  return (
    <FilePickerButton
      accept=".fit,application/vnd.ant.fit,application/octet-stream"
      onPick={(file) => uploader.uploadFile({ size: file.size, read: () => file.arrayBuffer() })}
      disabled={uploader.isUploading}
      aria-label={t('home.activities.upload.actionLabel')}
      className="rounded text-sm font-medium text-primary hover:underline focus-ring touch-target disabled:opacity-60 disabled:no-underline"
      data-testid="home-upload-action"
    >
      {t('home.activities.upload.action')}
    </FilePickerButton>
  );
}

/**
 * The line under the title while a file goes up and once it has: a polite
 * status while it travels and when it landed, an alert when it was refused.
 * Nothing before the first upload.
 */
export function UploadActivityStatus({ uploader }: { uploader: UseActivityUploadResult }) {
  const { t } = useTranslation();
  if (uploader.isUploading) {
    return (
      <p role="status" data-testid="home-upload-status" className="mb-2 text-sm text-on-surface-variant">
        {t('home.activities.upload.uploading')}
      </p>
    );
  }
  const { outcome } = uploader;
  if (outcome === null) return null;
  if (outcome.kind === 'uploaded') {
    return (
      <p role="status" data-testid="home-upload-status" className="mb-2 text-sm text-on-surface-variant">
        {describeActivityUpload(outcome, t)}
      </p>
    );
  }
  return (
    <p
      role="alert"
      data-testid="home-upload-failed"
      className="mb-2 rounded-lg bg-error/15 px-3 py-2 text-sm text-on-error-container"
    >
      {describeActivityUpload(outcome, t)}{' '}
      <button
        type="button"
        onClick={uploader.dismiss}
        className="rounded font-medium underline underline-offset-2 focus-ring touch-target"
      >
        {t('common.close')}
      </button>
    </p>
  );
}
