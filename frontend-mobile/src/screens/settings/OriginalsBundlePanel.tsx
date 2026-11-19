// ABOUTME: The originals bundle on the mobile privacy screen — ask for every kept original file as one ZIP, optionally sealed
// ABOUTME: Follows the build, then downloads it straight to a cache file (from its signed URL when the server gives one) and shares it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { Text, View } from 'react-native';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { File, Paths } from 'expo-file-system';
import * as Sharing from 'expo-sharing';
import { ENDPOINTS, type OriginalsBundle } from '@pierre/api-client';
import { parseOriginalsSignedDownload } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
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
import { Button, Input } from '../../components/ui';
import { authStorage, originalsApi } from '../../services/api';
import { getApiUrl } from '../../services/apiUrl';

/**
 * The error a signed download that does not parse is thrown with. A code, not
 * a sentence: the athlete reads the section's translated failure, never this.
 */
const ORIGINALS_DOWNLOAD_SHAPE = 'originals-download-shape';

const BUNDLE_FILE_STEM = 'dravr-originals';

/** The first byte of a downloaded file: `{` when the server answered a signed URL as JSON. */
function startsWithJson(file: File): boolean {
  const handle = file.open();
  try {
    return handle.readBytes(1)[0] === '{'.charCodeAt(0);
  } finally {
    handle.close();
  }
}

/**
 * Download a ready bundle straight to a cache file and open the share sheet on
 * it. The bundle can be gigabytes, so it never passes through a JavaScript
 * string: the server answers either the file itself, or a short signed URL as
 * JSON, which is then downloaded without the session. The files leave the
 * cache whether or not the share worked.
 */
async function downloadAndShare(bundle: OriginalsBundle): Promise<void> {
  const extension = bundle.sealed ? 'zip.age' : 'zip';
  const name = `${BUNDLE_FILE_STEM}-${bundle.createdAt.slice(0, 10)}.${extension}`;
  const answer = new File(Paths.cache, `${name}.answer`);
  const target = new File(Paths.cache, name);
  const token = await authStorage.getToken();
  try {
    await File.downloadFileAsync(
      `${getApiUrl()}${ENDPOINTS.USER.EXPORT_ORIGINALS_DOWNLOAD(bundle.bundleId)}`,
      answer,
      { headers: token ? { Authorization: `Bearer ${token}` } : {}, idempotent: true },
    );
    if (startsWithJson(answer)) {
      const signed = parseOriginalsSignedDownload(JSON.parse(answer.textSync()));
      if (!signed) throw new Error(ORIGINALS_DOWNLOAD_SHAPE);
      await File.downloadFileAsync(signed.url, target, { idempotent: true });
    } else {
      answer.move(target);
    }
    await Sharing.shareAsync(target.uri, {
      mimeType: bundle.sealed ? 'application/octet-stream' : 'application/zip',
    });
  } finally {
    if (answer.exists) answer.delete();
    if (target.exists) target.delete();
  }
}

export function OriginalsBundlePanel(): React.JSX.Element {
  const { t, i18n } = useTranslation();
  const [passphrase, setPassphrase] = useState('');
  const [failure, setFailure] = useState<string | null>(null);

  // The latest bundle, so a page left while it built finds it again. Every
  // answer about a bundle is written into this one entry: the client keeps a
  // query fresh for minutes, so a panel mounted again after a request would
  // otherwise read the `null` it cached before the athlete asked.
  const queryClient = useQueryClient();
  const remember = (seen: OriginalsBundle): OriginalsBundle => {
    queryClient.setQueryData(QUERY_KEYS.account.originalsLatest(), seen);
    return seen;
  };
  const latest = useQuery({
    queryKey: QUERY_KEYS.account.originalsLatest(),
    queryFn: () => originalsApi.getLatestOriginals(),
  });
  const current = latest.data ?? null;
  const followed = useQuery({
    queryKey: ['originals-bundle', current?.bundleId],
    queryFn: async () => remember(await originalsApi.getOriginals(current!.bundleId)),
    enabled: originalsStillBuilding(current),
    refetchInterval: (query) =>
      originalsStillBuilding(query.state.data ?? current) ? ORIGINALS_POLL_MS : false,
  });
  const bundle = followed.data ?? current;

  // The failure is said here and goes no further, as the export's own is.
  const request = useMutation({
    mutationFn: async () => {
      setFailure(null);
      try {
        remember(await originalsApi.requestOriginals(passphrase === '' ? undefined : passphrase));
        setPassphrase('');
      } catch (err: unknown) {
        setFailure(t(originalsRefusalKey(apiErrorStatus(err), exportRefusalCode(err))));
      }
    },
  });

  const download = useMutation({
    mutationFn: async (ready: OriginalsBundle) => {
      setFailure(null);
      try {
        await downloadAndShare(ready);
      } catch {
        setFailure(t('dataExport.originalsFailed'));
      }
    },
  });

  const sealOk = sealPassphraseAccepted(passphrase);
  const building = originalsStillBuilding(bundle);
  const expires = bundle ? new Date(bundle.expiresAt).toLocaleDateString(i18n.language) : '';

  return (
    <View className="gap-3 px-4 pt-4" testID="originals-bundle-panel">
      <Text className="text-base font-medium text-text-primary">{t('dataExport.originalsTitle')}</Text>
      <Text className="text-sm text-text-secondary">{t('dataExport.originalsDescription')}</Text>
      <Input
        label={t('dataExport.originalsSealLabel')}
        secureTextEntry
        showPasswordToggle
        autoCapitalize="none"
        autoCorrect={false}
        value={passphrase}
        onChangeText={setPassphrase}
        editable={!building && !request.isPending}
        error={sealOk ? undefined : t('dataExport.originalsSealTooShort')}
        testID="originals-passphrase"
      />
      <Text className="text-xs text-text-secondary">{t('dataExport.originalsSealHint')}</Text>
      <Button
        title={t('dataExport.originalsAction')}
        variant="secondary"
        onPress={() => request.mutate()}
        disabled={!sealOk || building || request.isPending}
        testID="originals-request"
      />
      {bundle && (
        <View className="gap-2" testID="originals-state">
          <Text className="text-sm text-text-primary" accessibilityLiveRegion="polite">
            {t(originalsStateKey(bundle), { files: bundle.files ?? 0, date: expires })}
          </Text>
          {bundle.sealed && bundle.status === 'ready' && (
            <Text className="text-sm text-text-secondary">{t('dataExport.originalsSealed')}</Text>
          )}
          {originalsIncomplete(bundle) && (
            <Text className="text-sm text-text-secondary" testID="originals-incomplete">
              {t('dataExport.originalsIncomplete')}
            </Text>
          )}
          {bundle.status === 'ready' && (
            <Button
              title={t('dataExport.originalsDownload')}
              onPress={() => download.mutate(bundle)}
              disabled={download.isPending}
              testID="originals-download"
            />
          )}
        </View>
      )}
      {failure && (
        <Text className="text-sm text-error" testID="originals-failure">
          {failure}
        </Text>
      )}
    </View>
  );
}
