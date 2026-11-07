// ABOUTME: Data export on the mobile privacy screen — the athlete saves their own data as one JSON file (Law 25 s.27)
// ABOUTME: Downloads straight to a file in the app cache, opens the share sheet, and says when the server sent a partial file

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { useMutation } from '@tanstack/react-query';
import { File, Paths } from 'expo-file-system';
import * as Sharing from 'expo-sharing';
import { ENDPOINTS } from '@pierre/api-client';
import { useTranslation } from '@pierre/i18n';
import {
  EXPORT_CUT_SHORT,
  EXPORT_TAIL_BYTES,
  exportPartialKey,
  readExportTail,
  type ExportTail,
} from '@pierre/ui-logic';
import { Button, Section } from '../../components/ui';
import { authStorage } from '../../services/api';
import { getApiUrl } from '../../services/apiUrl';

const EXPORT_FILE_STEM = 'dravr-export';

/**
 * The document's closing fields, read from the file's last bytes; `null` when
 * the download does not hold the document's end. A server that goes away part
 * of the way through leaves iOS reporting a finished download of the bytes it
 * got, and the file is too large to parse just to find that out.
 */
function readDownloadedTail(file: File): ExportTail | null {
  const handle = file.open();
  try {
    handle.offset = Math.max(0, (handle.size ?? 0) - EXPORT_TAIL_BYTES);
    const tail = handle.readBytes(EXPORT_TAIL_BYTES);
    return readExportTail(String.fromCharCode(...tail));
  } finally {
    handle.close();
  }
}

/**
 * Download the export straight to a file in the cache and open the share sheet
 * on it. The file can reach half a gigabyte, so it never passes through a
 * JavaScript string. It is the athlete's whole history, so it leaves the cache
 * whether or not the share worked, and a file a killed app left behind is
 * written over. Answers the sentence a partial file was shared with, or
 * `null` for a complete one.
 */
async function downloadAndShare(): Promise<string | null> {
  const day = new Date().toISOString().slice(0, 10);
  const file = new File(Paths.cache, `${EXPORT_FILE_STEM}-${day}.json`);
  const token = await authStorage.getToken();
  try {
    await File.downloadFileAsync(`${getApiUrl()}${ENDPOINTS.USER.EXPORT}`, file, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      idempotent: true,
    });
    const tail = readDownloadedTail(file);
    if (!tail) {
      throw new Error(EXPORT_CUT_SHORT);
    }
    await Sharing.shareAsync(file.uri, { mimeType: 'application/json' });
    return exportPartialKey(tail);
  } finally {
    if (file.exists) file.delete();
  }
}

/** Whether a refused download was the hourly limit or a full server. */
function wasLimited(err: unknown): boolean {
  return err instanceof Error && /\b(429|503)\b/.test(err.message);
}

export function DataExportSection(): React.JSX.Element {
  const { t } = useTranslation();
  const [failure, setFailure] = useState<string | null>(null);
  const [partial, setPartial] = useState<string | null>(null);

  // The failure is said here and goes no further: a rejected mutation also
  // reaches the app's global toast, which reads a native download's error as a
  // lost connection, so a refused export would say both at once.
  const mutation = useMutation({
    mutationFn: async () => {
      setFailure(null);
      setPartial(null);
      try {
        // A partial file is still the athlete's data: it is shared, and the
        // section says what it lacks.
        const partialKey = await downloadAndShare();
        setPartial(partialKey && t(partialKey));
      } catch (err: unknown) {
        setFailure(t(wasLimited(err) ? 'dataExport.rateLimited' : 'dataExport.failed'));
      }
    },
  });

  return (
    <Section
      title={t('dataExport.title')}
      description={t('dataExport.description')}
      testID="privacy-section-data-export"
    >
      <View className="gap-3 px-4">
        <Button
          title={mutation.isPending ? t('dataExport.preparing') : t('dataExport.action')}
          variant="secondary"
          onPress={() => mutation.mutate()}
          disabled={mutation.isPending}
          testID="data-export-action"
        />
        {partial && (
          <Text
            className="text-sm text-text-primary"
            accessibilityRole="text"
            accessibilityLiveRegion="polite"
            testID="data-export-partial"
          >
            {partial}
          </Text>
        )}
        {failure && (
          <Text className="text-sm text-error" testID="data-export-failure">
            {failure}
          </Text>
        )}
      </View>
    </Section>
  );
}
