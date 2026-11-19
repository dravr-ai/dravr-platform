// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The invite deep link — dravr://groups/join/<code> asks the server to hold the invite, then lands on Home
// ABOUTME: Home's join strip is where the athlete confirms the join; the link never sends /group join in a thread (carnet#1648)

import React, { useEffect, useState } from 'react';
import { Redirect, useLocalSearchParams } from 'expo-router';
import { ActivityIndicator, View } from 'react-native';
import Toast from 'react-native-toast-message';
import { useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { describeApiError } from '@pierre/ui-logic';
import { useThemeColors } from '../../../src/constants/theme';
import { CHAT_LIST_ROUTE, HOME_ROUTE } from '../../../src/navigation/routes';
import { groupsApi } from '../../../src/services/api';

/**
 * Hold a group invite from a link.
 *
 * The server keeps the code against the signed-in account (`PUT
 * /api/user/group-invite`), and the athlete joins from the strip on Home with
 * one press: a link is not consent to share training. A link whose code the
 * server refuses toasts the server's reason and still lands on Home. A link
 * with no code has nothing to hold, so it lands on the conversation list.
 *
 * The screen sits outside `(auth)`, so the root layout's guard bounces an
 * unauthenticated visitor to login and brings them back here after sign-in.
 */
export default function JoinGroupByInviteLink() {
  const { code } = useLocalSearchParams<{ code?: string }>();
  const trimmed = typeof code === 'string' ? code.trim() : '';
  const { t } = useTranslation();
  const colors = useThemeColors();
  const queryClient = useQueryClient();
  const [held, setHeld] = useState(false);

  useEffect(() => {
    if (!trimmed) return;
    let cancelled = false;
    groupsApi
      .holdInvite(trimmed)
      .catch((error: unknown) => {
        Toast.show({
          type: 'error',
          text1: t('common.error'),
          text2: describeApiError(error, { t, fallbackKey: 'app.somethingWentWrongRetry' }),
        });
      })
      .finally(() => {
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groupInvite.held() });
        if (!cancelled) setHeld(true);
      });
    return () => {
      cancelled = true;
    };
    // The hold is made once per code; `t` changing language must not repeat it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trimmed]);

  if (!trimmed) {
    return <Redirect href={CHAT_LIST_ROUTE} />;
  }
  if (held) {
    return <Redirect href={HOME_ROUTE} />;
  }
  return (
    <View className="flex-1 items-center justify-center bg-background-primary" testID="invite-link-holding">
      <ActivityIndicator color={colors.tokens.primary} />
    </View>
  );
}
