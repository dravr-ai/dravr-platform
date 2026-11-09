// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Uploading an export start to finish, for both clients: open the upload or read back the one waiting, send the bytes, finish
// ABOUTME: The bytes go to the store's session or whole through the API, as the server's answer says

import type { ArchiveImport, ImportUploadTarget } from '@pierre/shared-types';
import type { ImportsApi } from '../domains/imports';
import { uploadToSession, type UploadBytes } from './resumableUpload';

/** What uploading an export takes. */
export interface ExportUploadOptions {
  api: Pick<ImportsApi, 'startUpload' | 'getUploadSession' | 'writeUpload' | 'finishUpload'>;
  source: string;
  file: UploadBytes;
  /** The import already waiting for this upload, to resume it; `null` opens a new one. */
  waitingImportId: string | null;
  /** Told once the import exists, so a screen can show it. */
  onOpened?: () => void;
  /** Told how many bytes the store holds, as that grows. */
  onProgress?: (committed: number, total: number) => void;
  /** Stops the upload; a session stays resumable. */
  signal?: AbortSignal;
  /** The HTTP client for a session; the platform's `fetch` unless a test says otherwise. */
  fetchImpl?: typeof fetch;
  /** Waits between attempts on a session; a timer unless a test says otherwise. */
  wait?: (ms: number) => Promise<void>;
}

/**
 * Upload an export start to finish: open the upload or read back the one that
 * waits, send the bytes where it says, and tell the server they landed. Rejects
 * with the failure of whichever step stopped, and always releases the file.
 */
export async function uploadExport(options: ExportUploadOptions): Promise<ArchiveImport> {
  const { api, source, file, waitingImportId, onOpened, onProgress, signal, fetchImpl, wait } = options;
  try {
    let importId = waitingImportId;
    let target: ImportUploadTarget;
    if (importId === null) {
      const started = await api.startUpload(source, file.size);
      importId = started.import.id;
      target = started.upload;
      onOpened?.();
    } else {
      target = await api.getUploadSession(source, importId);
    }
    if (target.kind === 'session') {
      await uploadToSession({ sessionUrl: target.url, file, onProgress, signal, fetchImpl, wait });
    } else {
      const total = file.size;
      await api.writeUpload(source, importId, await file.read(0, total), (sent) => onProgress?.(sent, total));
    }
    return await api.finishUpload(source, importId);
  } finally {
    file.close?.();
  }
}
