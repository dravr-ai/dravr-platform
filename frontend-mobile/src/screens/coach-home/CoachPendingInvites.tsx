// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Coach Home on the phone before anyone joined — each group nobody has joined yet whose invite still works, as one true row
// ABOUTME: Mirrors the web CoachPendingInvites: copy the link again or write in the group; with no such group the empty state stands

import React, { useState } from 'react';
import { ActivityIndicator, Alert, Text, View } from 'react-native';
import { useRouter } from 'expo-router';
import { useQueries } from '@tanstack/react-query';
import * as Clipboard from 'expo-clipboard';
import type { CoachRosterGroup, GroupInvite } from '@pierre/shared-types';
import { QUERY_KEYS } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { formatDate, formatMessageTime, GROUP_ROOM_KEYS } from '@pierre/chat-utils';
import { COACH_PENDING_INVITE_KEYS, groupsAwaitingAthlete, usableMemberInvite } from '@pierre/ui-logic';
import { Button, EmptyState } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { inviteLink } from '../../constants/inviteLink';
import { useOpenGroupRoom } from '../../hooks/useCoachPlan';
import { threadHref } from '../../navigation/routes';
import { groupsApi } from '../../services/api';
import { AthleteAvatar, GroupLabel } from './parts';

/** A group nobody has joined, with the invite its athlete can still join with. */
interface PendingInvite {
  group: CoachRosterGroup;
  invite: GroupInvite;
}

/**
 * What a coach whose groups are all still empty sees on their Home: one row
 * per group whose athlete has not joined yet — the group, « invitation sent,
 * not joined yet », the time it was sent, why it is there, and the two things
 * to do about it. A row is shown only while its link still works.
 */
export function CoachPendingInvites({ groups }: { groups: readonly CoachRosterGroup[] }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const awaiting = groupsAwaitingAthlete(groups);
  const reads = useQueries({
    queries: awaiting.map((group) => ({
      queryKey: QUERY_KEYS.groups.invites(group.id),
      queryFn: () => groupsApi.listInvites(group.id),
    })),
  });
  const now = new Date();
  const pending: PendingInvite[] = awaiting.flatMap((group, i) => {
    const invite = usableMemberInvite(reads[i]?.data?.invites ?? [], now);
    return invite ? [{ group, invite }] : [];
  });

  if (reads.some((read) => read.isLoading)) {
    return (
      <View className="items-center py-10" accessibilityLabel={t('coachHome.loading')}>
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  }
  if (pending.length === 0) {
    return <EmptyState testID="coach-home-empty">{t('coachHome.noAthletes')}</EmptyState>;
  }

  return (
    <View testID="coach-pending-invites">
      <GroupLabel className="px-4 pb-1 pt-4">{t('coachHome.queue.today')}</GroupLabel>
      {pending.map((entry) => (
        <PendingInviteRow key={entry.group.id} entry={entry} />
      ))}
      <Text className="px-4 pt-4 text-sm text-text-secondary">{t(COACH_PENDING_INVITE_KEYS.after)}</Text>
    </View>
  );
}

function PendingInviteRow({ entry }: { entry: PendingInvite }) {
  const { t, language } = useTranslation();
  const router = useRouter();
  const { openRoom, pendingRoomAthleteId } = useOpenGroupRoom();
  const [copied, setCopied] = useState(false);
  const { group, invite } = entry;
  const roomKey = `invite:${group.id}`;

  const copy = async () => {
    try {
      await Clipboard.setStringAsync(inviteLink(invite.code));
      setCopied(true);
    } catch {
      Alert.alert(t('common.error'), t(COACH_PENDING_INVITE_KEYS.copyFailed));
    }
  };
  const writeToGroup = () => {
    openRoom(roomKey, group.id)?.then(
      (room) => router.push(threadHref(room.id)),
      () => Alert.alert(t('common.error'), t(GROUP_ROOM_KEYS.openError)),
    );
  };
  const opening = pendingRoomAthleteId === roomKey;

  return (
    <View className="px-4 py-3" testID={`coach-pending-invite-${group.id}`}>
      <View className="flex-row items-center gap-3">
        <AthleteAvatar userId={group.id} name={group.name} muted />
        <View className="min-w-0 flex-1">
          <Text className="text-sm font-semibold text-text-primary" numberOfLines={1}>
            {group.name}
          </Text>
          <Text className="text-sm text-text-secondary" numberOfLines={1} testID="coach-pending-invite-status">
            {t(COACH_PENDING_INVITE_KEYS.status)}
          </Text>
        </View>
        <Text className="font-mono text-xs text-text-secondary">{formatMessageTime(invite.created_at, language)}</Text>
      </View>
      <View className="mt-3 pl-12">
        <Text className="text-xs font-medium text-text-secondary">{t('coachHome.detail.why')}</Text>
        <Text className="mt-0.5 text-sm text-text-primary">
          {invite.expires_at
            ? t(COACH_PENDING_INVITE_KEYS.why, { date: formatDate(invite.expires_at, language) })
            : t(COACH_PENDING_INVITE_KEYS.whyNoExpiry)}
        </Text>
        <View className="mt-3 gap-2">
          <Button
            title={copied ? t(COACH_PENDING_INVITE_KEYS.copied) : t(COACH_PENDING_INVITE_KEYS.copyAgain)}
            onPress={() => void copy()}
            fullWidth
            testID="coach-pending-invite-copy"
          />
          <Button
            title={opening ? t('coachHome.action.openingGroup') : t('coachHome.action.writeToGroup')}
            variant="ghost"
            onPress={writeToGroup}
            disabled={opening}
            fullWidth
            testID="coach-pending-invite-write"
          />
        </View>
      </View>
    </View>
  );
}
