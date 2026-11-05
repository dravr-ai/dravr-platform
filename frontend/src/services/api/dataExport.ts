// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete's data export as the browser downloads it, a blob named for the day
// ABOUTME: Web only: the mobile app downloads the same route straight to a file, past the JavaScript heap

import { ENDPOINTS } from '@pierre/api-client';
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
};
