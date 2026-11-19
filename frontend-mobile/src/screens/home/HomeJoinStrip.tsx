// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's join strip — an invite the server holds for this account, with the one button that joins it (carnet#1650)
// ABOUTME: No silent join on the phone: the button is the consent moment; a full group says so and offers a retry

import React, { useCallback, useState } from 'react';
import { Text, View } from 'react-native';
import { useFocusEffect, useRouter } from 'expo-router';
import Toast from 'react-native-toast-message';
import { useTranslation } from '@pierre/i18n';
import { inviteShares, joinStripFull, joinStripJoin, joinStripTitle } from '@pierre/ui-logic';
import { Button } from '../../components/ui';
import { useHeldGroupInvite } from '../../hooks/useHeldGroupInvite';
import { threadHref } from '../../navigation/routes';

/** What the strip says about the last join attempt. */
type Attempt = 'idle' | 'full' | 'failed';

/**
 * One strip on an athlete's Home while the server holds a group invite for
 * them: who invites them to which group, what joining shares, and the filled
 * « Join » button. Nothing is drawn without a held invite. A group that was
 * full keeps its hold, so the strip stays with « try again »; a refusal for
 * good drops the hold and the server's own message is toasted.
 */
export function HomeJoinStrip() {
  const { t } = useTranslation();
  const router = useRouter();
  const { held, refetch, join, isJoining } = useHeldGroupInvite(true);
  const [attempt, setAttempt] = useState<Attempt>('idle');

  // Read again on every later focus, so an invite taken from a link opened
  // while Home was out of sight shows up. The first focus is the mount's read.
  useFocusEffect(
    useCallback(() => {
      refetch();
    }, [refetch]),
  );

  if (held === null) return null;

  // The inviter by name when they have one; what joining shares is said of
  // the group's coach, never the inviter (ADR-002).
  const title = joinStripTitle(held);
  const shares = inviteShares(held);
  const fullWords = joinStripFull(held);
  const joinWords = joinStripJoin(held);
  const full = attempt === 'full' || (attempt === 'idle' && held.state === 'group_full');

  const onJoin = () => {
    setAttempt('idle');
    join().then(
      (response) => {
        if (response.outcome === 'joined' && response.conversation_id) {
          router.push(threadHref(response.conversation_id));
        } else if (response.outcome === 'joined') {
          Toast.show({ type: 'success', text1: t('humanCoach.joinStrip.joined', { group: response.group_name }) });
        } else if (response.outcome === 'refused') {
          if (response.reason === 'group_full') setAttempt('full');
          else Toast.show({ type: 'error', text1: t('common.error'), text2: response.message });
        }
      },
      () => setAttempt('failed'),
    );
  };

  return (
    <View className="mx-4 gap-3 rounded-lg border border-border-faint bg-surface-container-low p-4" testID="home-join-strip">
      <Text accessibilityRole="header" className="text-sm font-semibold text-text-primary">
        {t(title.key, title.values)}
      </Text>
      <Text className="text-sm text-text-secondary">{t(shares.key, shares.values)}</Text>
      {full && (
        <Text className="text-sm text-text-primary" testID="home-join-strip-full">
          {t(fullWords.key, fullWords.values)}
        </Text>
      )}
      {attempt === 'failed' && (
        <Text className="text-sm text-text-primary" testID="home-join-strip-failed">
          {t('humanCoach.joinStrip.failed')}
        </Text>
      )}
      <Button
        title={
          isJoining
            ? t('humanCoach.joinStrip.joining')
            : full || attempt === 'failed'
              ? t('humanCoach.joinStrip.retry')
              : t(joinWords.key, joinWords.values)
        }
        onPress={onJoin}
        disabled={isJoining}
        fullWidth
        testID="home-join-strip-join"
      />
    </View>
  );
}
