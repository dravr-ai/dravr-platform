// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's data export and originals bundle as the browser downloads them, each a file named for the day
// ABOUTME: Web only: the mobile app downloads the same routes straight to a file, past the JavaScript heap

import { ENDPOINTS, type OriginalsBundle } from '@pierre/api-client';
import { parseOriginalsSignedDownload } from '@pierre/shared-types';
import { pierreApi } from './client';

/** The athlete's data export, named for the day it was taken. */
export interface UserDataExportFile {
  fileName: string;
  /**
   * The `dravr-user-export` JSON document. A blob because the file can reach
   * half a gigabyte, past what a JavaScript string can hold.
   */
  content: Blob;
}

export const dataExportApi = {
  async exportMyData(): Promise<UserDataExportFile> {
    const response = await pierreApi.axios.get<Blob>(ENDPOINTS.USER.EXPORT, {
      responseType: 'blob',
      timeout: 0,
    });
    const day = new Date().toISOString().slice(0, 10);
    return { fileName: `dravr-export-${day}.json`, content: response.data };
  },

  /**
   * Where the browser saves a ready originals bundle from: a short signed URL
   * the storage serves straight (the deployed answer), or the file itself
   * when the server signs none, named as the server named it.
   */
  async downloadOriginals(
    bundle: OriginalsBundle,
  ): Promise<{ kind: 'url'; url: string } | { kind: 'file'; file: UserDataExportFile }> {
    const response = await pierreApi.axios.get<Blob>(
      ENDPOINTS.USER.EXPORT_ORIGINALS_DOWNLOAD(bundle.bundleId),
      { responseType: 'blob', timeout: 0 },
    );
    const type = String(response.headers['content-type'] ?? '');
    if (type.startsWith('application/json')) {
      const signed = parseOriginalsSignedDownload(JSON.parse(await response.data.text()));
      if (!signed) {
        throw new Error('the originals download answered with a body that does not match its contract');
      }
      return { kind: 'url', url: signed.url };
    }
    const disposition = String(response.headers['content-disposition'] ?? '');
    const named = /filename="([^"]+)"/.exec(disposition)?.[1];
    const day = bundle.createdAt.slice(0, 10);
    const fileName = named ?? `dravr-originals-${day}.${bundle.sealed ? 'zip.age' : 'zip'}`;
    return { kind: 'file', file: { fileName, content: response.data } };
  },
};
