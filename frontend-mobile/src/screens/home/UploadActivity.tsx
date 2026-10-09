// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The recent-activities section's upload of a completed workout's .fit file — the action, and the line saying how it went
// ABOUTME: The system document picker hands over the file; a refusal is said by its reason, never by the server's prose

import React from 'react';
import { Pressable, Text, View } from 'react-native';
import * as DocumentPicker from 'expo-document-picker';
import { File } from 'expo-file-system';
import { useTranslation } from '@pierre/i18n';
import { describeActivityUpload, type UseActivityUploadResult } from '@pierre/ui-logic';

/**
 * Ask the system for one file and upload it. `.fit` has no registered type
 * on either platform, so the picker offers every file and the server says
 * whether it is a completed workout.
 */
async function pickAndUpload(uploader: UseActivityUploadResult) {
  const picked = await DocumentPicker.getDocumentAsync({ type: '*/*', copyToCacheDirectory: true, multiple: false });
  const asset = picked.canceled ? undefined : picked.assets?.[0];
  if (asset === undefined) return;
  uploader.uploadFile({ size: asset.size, read: () => new File(asset.uri).arrayBuffer() });
}

/** The upload action beside the section's title. */
export function UploadActivityAction({ uploader }: { uploader: UseActivityUploadResult }) {
  const { t } = useTranslation();
  return (
    <Pressable
      onPress={() => void pickAndUpload(uploader)}
      disabled={uploader.isUploading}
      accessibilityRole="button"
      accessibilityLabel={t('home.activities.upload.actionLabel')}
      accessibilityState={{ disabled: uploader.isUploading }}
      className="min-h-11 justify-center px-2"
      testID="home-upload-action"
    >
      <Text className={`text-sm font-medium text-primary ${uploader.isUploading ? 'opacity-60' : ''}`}>
        {t('home.activities.upload.action')}
      </Text>
    </Pressable>
  );
}

/**
 * The line under the title while a file goes up and once it has: a polite
 * status while it travels and when it landed, an alert when it was refused.
 * Nothing before the first upload.
 */
export function UploadActivityStatus({ uploader }: { uploader: UseActivityUploadResult }) {
  const { t } = useTranslation();
  const { outcome } = uploader;
  if (uploader.isUploading || outcome?.kind === 'uploaded') {
    return (
      <Text
        className="mx-4 mb-2 text-sm text-text-secondary"
        accessibilityLiveRegion="polite"
        testID="home-upload-status"
      >
        {uploader.isUploading || outcome === null
          ? t('home.activities.upload.uploading')
          : describeActivityUpload(outcome, t)}
      </Text>
    );
  }
  if (outcome === null) return null;
  return (
    <View
      className="mx-4 mb-2 flex-row flex-wrap items-center rounded-lg bg-error/15 px-3"
      accessibilityRole="alert"
      accessibilityLiveRegion="polite"
      testID="home-upload-failed"
    >
      <Text className="py-2 text-sm text-on-error-container">{describeActivityUpload(outcome, t)}</Text>
      <Pressable
        onPress={uploader.dismiss}
        accessibilityRole="button"
        className="ml-1 min-h-11 justify-center px-2"
        testID="home-upload-dismiss"
      >
        <Text className="text-sm font-medium text-on-error-container underline">{t('common.close')}</Text>
      </Pressable>
    </View>
  );
}
