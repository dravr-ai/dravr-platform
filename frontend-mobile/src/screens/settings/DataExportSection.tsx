// ABOUTME: Data export on the mobile privacy screen — the athlete saves their own data as one JSON file (Law 25 s.27)
// ABOUTME: Downloads straight to a file in the app cache, then opens the share sheet to save or send it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { useMutation } from '@tanstack/react-query';
import { File, Paths } from 'expo-file-system';
import * as Sharing from 'expo-sharing';
import { ENDPOINTS } from '@pierre/api-client';
import { useTranslation } from '@pierre/i18n';
import { Button, Section } from '../../components/ui';
import { authStorage } from '../../services/api';
import { getApiUrl } from '../../services/apiUrl';

const EXPORT_FILE_STEM = 'dravr-export';

/**
 * Download the export straight to a file in the cache and open the share sheet
 * on it. The file can reach half a gigabyte, so it never passes through a
 * JavaScript string. It is the athlete's whole history, so it leaves the cache
 * whether or not the share worked, and a file a killed app left behind is
 * written over.
 */
async function downloadAndShare(): Promise<void> {
  const day = new Date().toISOString().slice(0, 10);
  const file = new File(Paths.cache, `${EXPORT_FILE_STEM}-${day}.json`);
  const token = await authStorage.getToken();
  try {
    await File.downloadFileAsync(`${getApiUrl()}${ENDPOINTS.USER.EXPORT}`, file, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      idempotent: true,
    });
    await Sharing.shareAsync(file.uri, { mimeType: 'application/json' });
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

  const mutation = useMutation({
    mutationFn: downloadAndShare,
    onMutate: () => setFailure(null),
    onError: (err: unknown) => {
      setFailure(t(wasLimited(err) ? 'dataExport.rateLimited' : 'dataExport.failed'));
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
        {failure && (
          <Text className="text-sm text-error" testID="data-export-failure">
            {failure}
          </Text>
        )}
      </View>
    </Section>
  );
}
