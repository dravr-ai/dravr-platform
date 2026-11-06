// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The file step of an import whose export is a file the athlete holds: pick it, send it, follow the bytes, resume or cancel
// ABOUTME: Reads the picked file through a file handle, one chunk at a time, so a multi-gigabyte export is never held in memory

import React, { useEffect, useRef, useState } from 'react';
import { Text, View } from 'react-native';
import * as DocumentPicker from 'expo-document-picker';
import { File } from 'expo-file-system';
import { useMutation } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { uploadExport, type UploadBytes } from '@pierre/api-client';
import type { ArchiveImport } from '@pierre/shared-types';
import { uploadErrorKey, uploadPercent, type UploadSourceCopy } from '@pierre/ui-logic';
import { importsApi } from '../../services/api';
import { Button, Section } from '../../components/ui';

/** The file at `uri`, read a chunk at a time through one handle. */
function bytesOf(uri: string): UploadBytes {
  const file = new File(uri);
  const handle = file.open();
  return {
    size: file.size,
    read: async (start, end) => {
      handle.offset = start;
      return handle.readBytes(end - start);
    },
    close: () => handle.close(),
  };
}

interface UploadIntakeProps {
  copy: UploadSourceCopy;
  /** The source's latest import: one still waiting is resumed rather than opened anew. */
  current: ArchiveImport | null | undefined;
  /** The import changed: read it again. */
  onChanged: () => void;
}

export function UploadIntake({ copy, current, onChanged }: UploadIntakeProps) {
  const { t } = useTranslation();
  const { source } = copy;
  const [percent, setPercent] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const stop = useRef<AbortController | null>(null);
  const waiting = current?.status === 'awaiting_link' ? current : null;

  useEffect(() => () => stop.current?.abort(), []);

  const upload = useMutation({
    mutationFn: (uri: string) => {
      const controller = new AbortController();
      stop.current = controller;
      return uploadExport({
        api: importsApi,
        source,
        file: bytesOf(uri),
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

  const pick = async () => {
    const picked = await DocumentPicker.getDocumentAsync({
      type: [...copy.documentTypes],
      copyToCacheDirectory: true,
      multiple: false,
    });
    const asset = picked.canceled ? undefined : picked.assets?.[0];
    if (asset !== undefined) upload.mutate(asset.uri);
  };

  return (
    <Section title={t(copy.step2Title)} testID="import-upload">
      <Text className="text-sm text-text-secondary">{t(waiting ? copy.resumeHint : copy.pickHint)}</Text>
      {percent !== null ? (
        <View className="mt-3">
          <Text className="text-sm text-text-primary" accessibilityLiveRegion="polite" testID="import-upload-progress">
            {t(copy.uploading, { percent })}
          </Text>
          <Text className="text-xs text-text-secondary mt-2">{t(copy.keepOpen)}</Text>
        </View>
      ) : (
        <View className="mt-3">
          <Button
            title={t(waiting ? copy.resume : copy.pick)}
            onPress={() => void pick()}
            testID="import-upload-pick"
          />
        </View>
      )}
      {waiting && (
        <View className="mt-3">
          <Button
            title={t('providers.import.strava.cancel')}
            variant="secondary"
            onPress={() => cancel.mutate(waiting.id)}
            loading={cancel.isPending}
            testID="import-upload-cancel"
          />
        </View>
      )}
      {error && (
        <Text className="text-sm text-error mt-2" accessibilityRole="alert" testID="import-upload-error">
          {error}
        </Text>
      )}
    </Section>
  );
}
