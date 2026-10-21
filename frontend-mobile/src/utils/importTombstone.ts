// ABOUTME: The phone's answer to a provider connect refused over a deleted account's tombstone
// ABOUTME: Words the refusal with its date and offers to start fresh: clear the tombstone, then connect again

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { Alert } from 'react-native';
import {
  describeApiError,
  describeProviderTombstone,
  type ApiErrorTranslate,
  type ImportTombstoneRefusal,
} from '@pierre/ui-logic';
import { userApi } from '../services/api';

interface TombstonePrompt {
  refusal: ImportTombstoneRefusal;
  /** The brand the provider's row shows. */
  providerName: string;
  language: string;
  t: ApiErrorTranslate;
  /** Runs once the tombstone is cleared: the provider's connect, again. */
  onCleared: () => void;
}

/**
 * Tell the athlete the provider account belonged to a deleted Dravr account,
 * and offer to start fresh with it. Nothing reopens on its own: the clear runs
 * only on the athlete's tap, and the connect starts again only once it held.
 */
export function promptImportTombstoneClear({ refusal, providerName, language, t, onCleared }: TombstonePrompt): void {
  Alert.alert(providerName, describeProviderTombstone(refusal, providerName, { language, t }), [
    { text: t('common.cancel'), style: 'cancel' },
    {
      text: t('providers.connectTombstonedClear'),
      onPress: () => {
        userApi
          .clearImportTombstone(refusal.tombstoneId)
          .then(onCleared)
          .catch((err: unknown) => {
            Alert.alert(t('common.error'), describeApiError(err, { t, fallbackKey: 'errors.unknown' }));
          });
      },
    },
  ]);
}
