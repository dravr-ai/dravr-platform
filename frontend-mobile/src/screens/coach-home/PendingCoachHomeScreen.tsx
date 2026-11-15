// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A coach's Home on the phone while their coach access is pending — « Préparer ton coaching »: the seat, the athlete, a platform
// ABOUTME: Mirrors the web PendingCoachHome: rows ticked by GET /api/me/coach-setup; a grant turns the tab into the coach Home (carnet#1529)

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { ActivityIndicator, Alert, RefreshControl, ScrollView, Share, Text, View } from 'react-native';
import { Stack, useRouter } from 'expo-router';
import { Feather } from '@expo/vector-icons';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  COACH_SEAT_KEYS,
  COACH_SETUP_KEYS,
  ONBOARDING_INVITE_DAYS,
  QUERY_KEYS,
  coachPlatformName,
  coachSeatState,
  defaultGroupAgent,
  groupAgentChoices,
} from '@pierre/shared-constants';
import type { CoachSetupGroup, CoachSetupResponse, CoachSetupRow } from '@pierre/shared-types';
import { GROUP_ROOM_KEYS } from '@pierre/chat-utils';
import { COACH_PENDING_INVITE_KEYS, seatLineDay, usableMemberInvite } from '@pierre/ui-logic';
import { BrandLockup, Button, EmptyState, Input, Row } from '../../components/ui';
import { CoachAccessRequest, CoachPlatformList } from '../../components/CoachAccessRequest';
import { useThemeColors } from '../../constants/theme';
import { inviteLink } from '../../constants/inviteLink';
import { useSeatAfterPlatformLink } from '../../hooks/coachAccessHooks';
import { useOpenGroupRoom } from '../../hooks/useCoachPlan';
import { useCoachGroupSetup, useCreateInvite, useGroupInvites } from '../../hooks/useGroups';
import { useProviderConnected } from '../../hooks/useHome';
import { CONNECTIONS_ROUTE, threadHref } from '../../navigation/routes';
import { coachesApi, userApi } from '../../services/api';

interface PendingCoachHomeScreenProps {
  /** The "Mes athlètes / Mon entraînement" switch, for a coach who trains too; none for one who does not. */
  scopeSwitch?: React.ReactNode;
}

/**
 * The Home tab of a coach whose coach access is pending (P1·D, carnet#1529),
 * in place of the athlete's — mirroring the web. « Préparer ton coaching »
 * lists the three steps the server ticks: the coach seat, with the request or
 * its dated state (the one filled button); the athlete invited, which makes
 * the group the onboarding way when there is none, then shares its link;
 * TrainingPeaks and Intervals.icu as the same rows Group info shows (D3).
 * Under it, the coach's groups, where the agent already answers.
 *
 * A grant flips the tab without signing out: the shell's grant refresh, or
 * {@link useSeatAfterPlatformLink} for a TrainingPeaks coach account, patches
 * the session and `HomeRoot` opens the coach Home.
 */
export function PendingCoachHomeScreen({ scopeSwitch }: PendingCoachHomeScreenProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const scrollRef = useRef<ScrollView>(null);
  const [refreshing, setRefreshing] = useState(false);
  const setup = useQuery({ queryKey: QUERY_KEYS.home.coachSetup(), queryFn: () => userApi.getCoachSetup() });
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const provider = useProviderConnected();
  useSeatAfterPlatformLink(provider.providers?.filter((p) => p.connected).map((p) => p.provider));

  const { refetch: refetchSetup } = setup;
  const { refetch: refetchRequest } = latest;
  const { refetch: refetchProviders } = provider;
  const onRefresh = useCallback(() => {
    setRefreshing(true);
    void Promise.allSettled([refetchSetup(), refetchRequest(), refetchProviders()]).finally(() => setRefreshing(false));
  }, [refetchSetup, refetchRequest, refetchProviders]);
  const scrollToTop = useCallback(() => scrollRef.current?.scrollTo({ y: 0, animated: true }), []);

  const scope = latest.isSuccess
    ? t(coachSeatState(latest.data.request) === 'pending' ? COACH_SETUP_KEYS.scopePending : COACH_SETUP_KEYS.scope)
    : null;

  let body: React.ReactNode;
  if (setup.data !== undefined) {
    body = (
      <View className="gap-8">
        <Checklist setup={setup.data} />
        <CoachGroups groups={setup.data.groups} />
      </View>
    );
  } else if (setup.isError) {
    body = (
      <EmptyState
        testID="pending-coach-home-failed"
        action={{ label: t('common.retry'), onPress: () => void refetchSetup(), testID: 'pending-coach-home-retry' }}
      >
        {t(COACH_SETUP_KEYS.loadFailed)}
      </EmptyState>
    );
  } else {
    body = (
      <View className="items-center py-10" accessibilityLabel={t('common.loading')}>
        <ActivityIndicator color={colors.tokens.primary} />
      </View>
    );
  }

  return (
    <View className="flex-1 bg-background-primary" testID="pending-coach-home">
      <Stack.Screen
        options={{
          headerTitle: () => <BrandLockup accessibilityLabel={t('nav.home')} onPress={scrollToTop} testID="home-title" />,
        }}
      />
      <ScrollView
        ref={scrollRef}
        // The system header and tab bar inset the page themselves.
        contentInsetAdjustmentBehavior="automatic"
        contentContainerClassName="pb-8"
        refreshControl={<RefreshControl refreshing={refreshing} onRefresh={onRefresh} tintColor={colors.tokens.primary} />}
        testID="pending-coach-home-scroll"
      >
        {/* The same 16 from the top as the coach Home and the athlete Home, so the pill stays under the finger. */}
        <View className="px-4 pb-2 pt-4 gap-2">
          {scopeSwitch ?? (
            <Text className="text-xl font-semibold text-text-primary" accessibilityRole="header">
              {t('coachHome.switch.athletes')}
            </Text>
          )}
          {scope !== null && (
            <Text className="text-xs text-text-secondary" testID="pending-coach-scope">
              {scope}
            </Text>
          )}
        </View>
        {body}
      </ScrollView>
    </View>
  );
}

/** « Préparer ton coaching »: the server's rows, in its order, with its count. */
function Checklist({ setup }: { setup: CoachSetupResponse }) {
  const { t } = useTranslation();
  const rows = [...setup.rows].sort((a, b) => a.order - b.order);
  return (
    <View className="px-4 pt-2 gap-2.5" testID="coach-setup">
      <View
        className="flex-row items-baseline gap-2"
        accessible
        accessibilityRole="header"
        accessibilityLabel={`${t(COACH_SETUP_KEYS.title)}, ${t(COACH_SETUP_KEYS.countAria, { done: setup.done_count, total: setup.total })}`}
      >
        <Text className="text-base font-semibold text-text-primary">{t(COACH_SETUP_KEYS.title)}</Text>
        <Text className="font-mono text-xs text-text-tertiary" testID="coach-setup-count">
          {t(COACH_SETUP_KEYS.count, { done: setup.done_count, total: setup.total })}
        </Text>
      </View>
      <View className="gap-1">
        {rows.map((row) => (
          <SetupRow key={row.id} row={row} setup={setup} />
        ))}
      </View>
    </View>
  );
}

/**
 * One row: its marker, its title (« · fait » once done) and what it holds.
 * The current row is a bordered box, never a Card (the Mobile Less ratchet).
 */
function SetupRow({ row, setup }: { row: CoachSetupRow; setup: CoachSetupResponse }) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const open = row.current && !row.done;
  let title: string;
  let content: React.ReactNode;
  switch (row.id) {
    case 'seat':
      title = t(COACH_SEAT_KEYS.panelTitle);
      content = row.done ? null : <CoachAccessRequest variant="row" />;
      break;
    case 'invite':
      title = t(COACH_SETUP_KEYS.inviteTitle);
      content = <InviteContent setup={setup} done={row.done} />;
      break;
    case 'platform':
      title = t(COACH_SETUP_KEYS.platformTitle);
      content = <PlatformContent setup={setup} done={row.done} />;
      break;
  }
  const marker = row.done
    ? 'bg-primary'
    : open
      ? 'border-2 border-primary'
      : 'border-[1.5px] border-border-strong';
  return (
    <View
      className={`flex-row gap-3 rounded-xl border px-4 ${open ? 'border-border-faint bg-surface-container-lowest py-3.5' : 'border-transparent py-3'}`}
      testID={`coach-setup-row-${row.id}`}
      accessibilityState={{ selected: open }}
    >
      <View className={`mt-px h-[22px] w-[22px] items-center justify-center rounded-full ${marker}`}>
        {row.done && <Feather name="check" size={13} color={colors.tokens.onPrimary} />}
      </View>
      <View className="min-w-0 flex-1 gap-1">
        <Text className="text-sm font-semibold text-text-primary" testID={`coach-setup-row-${row.id}-${row.done ? 'done' : open ? 'current' : 'open'}`}>
          {title}
          {row.done && <Text className="font-normal text-text-secondary">{` · ${t(COACH_SETUP_KEYS.done)}`}</Text>}
        </Text>
        {content}
      </View>
    </View>
  );
}

/**
 * The invite row: who joined and when, once someone has; else the group's
 * link to share; else, with no group yet, the group made the onboarding way —
 * named after the athlete, on the endurance generalist (D8).
 */
function InviteContent({ setup, done }: { setup: CoachSetupResponse; done: boolean }) {
  const { t, language } = useTranslation();
  if (done && setup.athlete) {
    const date = seatLineDay(setup.athlete.joined_at, language);
    return (
      <Text className="text-sm text-text-secondary" testID="coach-setup-athlete-joined">
        {setup.athlete.name
          ? t(COACH_SETUP_KEYS.inviteDone, { name: setup.athlete.name, date })
          : t(COACH_SETUP_KEYS.inviteDoneNoName, { date })}
      </Text>
    );
  }
  const group = setup.groups[0];
  return group ? <InviteShare group={group} /> : <CreateGroup />;
}

/** The coach's first group, made as onboarding makes it, from the athlete's name. */
function CreateGroup() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [athlete, setAthlete] = useState('');
  const agents = useQuery({ queryKey: QUERY_KEYS.coaches.list(), queryFn: () => coachesApi.list() });
  const agent = defaultGroupAgent(groupAgentChoices(agents.data?.agents ?? []));
  const { create, working, failed, group } = useCoachGroupSetup();
  // The checklist reads the group the server now holds, and its link.
  useEffect(() => {
    if (group) void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.home.coachSetup() });
  }, [group, queryClient]);
  const name = athlete.trim();

  return (
    <View className="gap-2">
      <Text className="text-sm text-text-secondary">{t(COACH_SETUP_KEYS.groupIntro)}</Text>
      <Input
        label={t(COACH_SETUP_KEYS.athleteLabel)}
        placeholder={t(COACH_SETUP_KEYS.athletePlaceholder)}
        value={athlete}
        maxLength={60}
        editable={!working && group === null}
        onChangeText={setAthlete}
        returnKeyType="done"
        testID="coach-setup-athlete"
      />
      <Button
        title={working ? t(COACH_SETUP_KEYS.creating) : t(COACH_SETUP_KEYS.create)}
        variant="secondary"
        onPress={() => {
          if (name !== '' && agent) void create(name, agent.id);
        }}
        disabled={working || group !== null || name === '' || !agent}
        fullWidth
        testID="coach-setup-create"
      />
      {failed && (
        <Text className="text-sm text-error" accessibilityRole="alert">
          {t(COACH_SETUP_KEYS.createFailed)}
        </Text>
      )}
    </View>
  );
}

/**
 * The group's link to share: an invite that still works says so, neutrally
 * (D19); with none, a new one is issued the onboarding way on the first share.
 */
function InviteShare({ group }: { group: CoachSetupGroup }) {
  const { t } = useTranslation();
  const { invites, isLoading } = useGroupInvites(group.id);
  const { createInvite, isPending } = useCreateInvite(group.id);
  const invite = usableMemberInvite(invites, new Date());

  const share = async () => {
    try {
      const code = invite?.code ?? (await createInvite({ expires_in_days: ONBOARDING_INVITE_DAYS })).code;
      await Share.share({ message: inviteLink(code) });
    } catch {
      Alert.alert(t('common.error'), t(COACH_PENDING_INVITE_KEYS.copyFailed));
    }
  };

  if (isLoading) return null;
  return (
    <View className="gap-2">
      {invite ? (
        <Text className="text-sm text-text-secondary" testID="coach-setup-invite-status">
          <Text className="font-medium text-text-primary">{group.name}</Text>
          {` · ${t(COACH_PENDING_INVITE_KEYS.status)}`}
        </Text>
      ) : (
        <Text className="text-sm text-text-secondary">
          {t(COACH_SETUP_KEYS.linkIntro, { group: group.name, days: ONBOARDING_INVITE_DAYS })}
        </Text>
      )}
      <Button
        title={t(COACH_SETUP_KEYS.shareLink)}
        variant="ghost"
        onPress={() => void share()}
        disabled={isPending}
        fullWidth
        testID="coach-setup-share-link"
      />
    </View>
  );
}

/**
 * The coaching-platform row: TrainingPeaks and Intervals.icu as the plain,
 * identical rows of Group info's seat block (D3), each opening Connections;
 * once one is linked, which.
 */
function PlatformContent({ setup, done }: { setup: CoachSetupResponse; done: boolean }) {
  const { t } = useTranslation();
  const router = useRouter();
  if (done && setup.platform) {
    return (
      <Text className="text-sm text-text-secondary" testID="coach-setup-platform-linked">
        {t(COACH_SETUP_KEYS.platformDone, { provider: coachPlatformName(setup.platform) })}
      </Text>
    );
  }
  return (
    <View className="gap-1" testID="coach-setup-platforms">
      <Text className="text-sm text-text-secondary">{t(COACH_SETUP_KEYS.platformBody)}</Text>
      <CoachPlatformList onOpenConnections={() => router.push(CONNECTIONS_ROUTE)} />
    </View>
  );
}

/** The coach's groups, each opening its room, and that the agent already answers there. */
function CoachGroups({ groups }: { groups: readonly CoachSetupGroup[] }) {
  const { t } = useTranslation();
  const router = useRouter();
  const { openRoom, pendingRoomAthleteId } = useOpenGroupRoom();
  if (groups.length === 0) return null;
  return (
    <View testID="coach-setup-rooms">
      <Text className="px-4 pb-1 text-base font-semibold text-text-primary" accessibilityRole="header">
        {t(groups.length === 1 ? COACH_SETUP_KEYS.groupsTitleOne : COACH_SETUP_KEYS.groupsTitleN)}
      </Text>
      {groups.map((group, index) => {
        const roomKey = `room:${group.id}`;
        return (
          <Row
            key={group.id}
            title={group.name}
            last={index === groups.length - 1}
            onPress={() =>
              openRoom(roomKey, group.id)?.then(
                (room) => router.push(threadHref(room.id)),
                () => Alert.alert(t('common.error'), t(GROUP_ROOM_KEYS.openError)),
              )
            }
            accessibilityState={{ busy: pendingRoomAthleteId === roomKey }}
            testID={`coach-setup-room-${group.id}`}
          />
        );
      })}
      <Text className="px-4 pt-2 text-xs text-text-secondary">{t(COACH_SETUP_KEYS.groupsNote)}</Text>
    </View>
  );
}
