// ABOUTME: Onboarding steps "Your group" and "Invite" (mobile) — the athlete, the group's name and agent on one screen, then the link to share
// ABOUTME: Mirrors the web OnboardingCoachGroup; coach access is never granted here (ADR-018), the seat card says what the server holds

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useEffect, useState } from 'react';
import { View, Text, ScrollView, Pressable, Share } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useRouter } from 'expo-router';
import { useQuery } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import QRCode from 'react-native-qrcode-svg';
import type { Agent, CoachingGroup } from '@pierre/shared-types';
import {
  BOREAL_LIGHT,
  ONBOARDING_INVITE_DAYS,
  QUERY_KEYS,
  canRequestCoachAccess,
  coachCategoryLabelKey,
  coachSeatMoment,
  coachSeatState,
  isCoachFacing,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button, Input, Row } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { useAuth } from '../../contexts/AuthContext';
import { useThemeColors } from '../../constants/theme';
import { coachesApi, userApi } from '../../services/api';
import { useCoachGroupSetup } from '../../hooks/useGroups';
import { COACH_GROUP_DONE_PREFIX, useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useCoachGroupCreated } from '../../hooks/useCoachGroupCreated';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { useOnboardingStatus } from '../../hooks/useOnboardingStatus';
import { useRecordOnboardingStep } from '../../hooks/useRecordOnboardingStep';
import { inviteLink } from '../../constants/inviteLink';
import { CoachAccessRequest } from '../../components/CoachAccessRequest';
import { HOME_ROUTE, threadHref } from '../../navigation/routes';

/** A QR is not themed: the phone camera needs dark modules on a light ground in both schemes. */
const QR_GROUND = BOREAL_LIGHT.surfaceContainerLowest;
const QR_INK = BOREAL_LIGHT.onSurface;

/**
 * The coach's group, then its invite (mobile): two segments of the progress
 * bar on one route, so the group just made and its link survive the step
 * changing (`useCoachGroupCreated` moves the bar on).
 *
 * "Your group" is one screen: the athlete, their goal if given, the group's
 * name filled in from them, and the agent the athletes talk to — defaulted,
 * with « Changer » opening the catalogue. The server decides whether the
 * coach is set as the group's coach.
 *
 * Where the invite leaves for follows the seat, as on the web: a coach whose
 * access is active (the server made them the group's coach — invited by the
 * team, or a TrainingPeaks coach account) opens their coach Home, where the
 * athlete they just invited waits as a row; one still without it goes to the
 * group's thread. A coach the team invited (server truth: `settled_by_invite`)
 * also reads on the bar that the invitation settled the role and the seat,
 * and on the group screen that the seat is active.
 */
export function OnboardingCoachGroupScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const router = useRouter();
  const { mark } = useOnboardingFlag(COACH_GROUP_DONE_PREFIX, user?.id);
  const { mark: markCreated } = useCoachGroupCreated(user?.id);
  const progress = useOnboardingProgress('coach_group');
  const colors = useThemeColors();
  const { data: onboardingStatus } = useOnboardingStatus(true);
  const settledByInvite = onboardingStatus?.settled_by_invite === true;
  const recordStep = useRecordOnboardingStep();
  const [athlete, setAthlete] = useState('');
  const [goal, setGoal] = useState('');
  // The group's name follows the athlete until the coach types their own.
  const [typedName, setTypedName] = useState<string | null>(null);
  const [pickedAgentId, setPickedAgentId] = useState<string | null>(null);
  const [changingAgent, setChangingAgent] = useState(false);
  const { create, working, failed, group: created, conversationId: threadId, inviteCode } = useCoachGroupSetup();
  const agents = useGroupAgents();
  const link = inviteCode === null ? null : inviteLink(inviteCode);

  const athleteName = athlete.trim();
  const goalText = goal.trim();
  const suggestedName =
    athleteName === ''
      ? ''
      : goalText === ''
        ? athleteName
        : t('onboarding.groupNameFromAthleteGoal', { athlete: athleteName, goal: goalText });
  const name = typedName ?? suggestedName;
  const agentId = pickedAgentId ?? defaultAgent(agents.list)?.id ?? null;
  const agent = agents.list.find((a) => a.id === agentId) ?? null;

  const ready = created !== null && threadId !== null && link !== null;
  useEffect(() => {
    if (ready) markCreated();
  }, [ready, markCreated]);

  const finish = async (status: 'complete' | 'skipped') => {
    recordStep('coach_group', status);
    await mark();
  };

  // The step is recorded first, so the layout no longer holds the coach in
  // onboarding; then the group's thread replaces the step. Nothing sits
  // beneath it, so the thread's Back falls back to the conversation list.
  const goToGroup = async (conversationId: string) => {
    await finish('complete');
    router.replace(threadHref(conversationId));
  };

  // Any active seat lands on the coach Home: the server made the coach the
  // group's coach only because they hold coach access.
  const toCoachHome = created?.coach_user_id != null;
  const goToCoachHome = async () => {
    await finish('complete');
    router.replace(HOME_ROUTE);
  };

  if (created && threadId && link) {
    return (
      <SafeAreaView className="flex-1 bg-surface">
        <ScrollView contentContainerClassName="py-10 px-4">
          <OnboardingProgressBar steps={progress} settledByInvite={settledByInvite} />
          <InviteShare
            group={created}
            link={link}
            athleteName={athleteName}
            toCoachHome={toCoachHome}
            onDone={() => void (toCoachHome ? goToCoachHome() : goToGroup(threadId))}
          />
        </ScrollView>
      </SafeAreaView>
    );
  }

  // Once the group exists its name and agent are fixed: a retry after a later
  // call failed can apply no edit, so the fields lock.
  const locked = working || created !== null;
  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="py-10" keyboardShouldPersistTaps="handled">
        <View className="px-4">
          <OnboardingProgressBar steps={progress} settledByInvite={settledByInvite} />
          {settledByInvite ? (
            <View
              className="mt-4 flex-row items-center gap-2 rounded-lg bg-success/10 px-3 py-2"
              testID="onboarding-group-seat-active"
            >
              <Feather name="check" size={16} color={colors.ink.success} />
              <Text className="flex-1 text-sm text-on-surface">{t('humanCoach.seatInvite.seatActive')}</Text>
            </View>
          ) : null}
          <Text className="mt-4 text-3xl font-display text-left text-on-surface">
            {t('onboarding.groupHeading')}
          </Text>
          <Text className="mt-3 text-sm text-on-surface-variant">{t('humanCoach.groupStepIntro')}</Text>

          <View className="mt-8 gap-6">
            <Input
              label={t('onboarding.groupAthleteLabel')}
              placeholder={t('onboarding.groupAthletePlaceholder')}
              value={athlete}
              maxLength={60}
              editable={!locked}
              onChangeText={setAthlete}
              testID="onboarding-group-athlete"
            />
            <Input
              label={t('onboarding.groupGoalLabel')}
              placeholder={t('onboarding.groupGoalPlaceholder')}
              value={goal}
              maxLength={80}
              editable={!locked}
              onChangeText={setGoal}
              testID="onboarding-group-goal"
            />
            <View>
              <Input
                label={t('onboarding.groupNameLabel')}
                placeholder={t('onboarding.groupNamePlaceholder')}
                value={name}
                maxLength={100}
                editable={!locked}
                onChangeText={setTypedName}
                testID="onboarding-group-name"
              />
              {typedName === null && suggestedName !== '' ? (
                <Text className="mt-1.5 text-xs text-on-surface-variant">{t('onboarding.groupNameHint')}</Text>
              ) : null}
            </View>
          </View>
        </View>

        <AgentField
          agents={agents}
          agent={agent}
          changing={changingAgent}
          disabled={locked}
          onChange={() => setChangingAgent(true)}
          onSelect={(id) => {
            setPickedAgentId(id);
            setChangingAgent(false);
          }}
        />

        <View className="mt-8 gap-3 px-4">
          {failed ? (
            <Text className="text-center text-sm text-error" accessibilityRole="alert">
              {t('onboarding.groupCreateFailed')}
            </Text>
          ) : null}
          <Button
            title={working ? t('onboarding.groupCreating') : t('onboarding.groupCreate')}
            onPress={() => {
              if (name.trim() !== '' && agentId !== null) void create(name, agentId, goalText);
            }}
            disabled={working || name.trim() === '' || agentId === null}
            testID="onboarding-group-create"
          />
          {created ? null : (
            <Button
              title={t('onboarding.groupLater')}
              variant="ghost"
              onPress={() => void finish('skipped')}
              disabled={working}
              testID="onboarding-group-later"
            />
          )}
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

/** The athlete-facing catalogue the group's agent is chosen from. */
function useGroupAgents() {
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
  });
  // Coach-facing agents are left out: the athletes talk to this one.
  const list: Agent[] = (data?.agents ?? []).filter((a) => !a.is_hidden && !isCoachFacing(a));
  return { list, isLoading, isError };
}

/**
 * The agent a new group starts with: the endurance generalist, which reads
 * load, recovery and key sessions whatever the sport, else the catalogue's
 * first. Mirrors the web step.
 */
function defaultAgent(agents: Agent[]): Agent | undefined {
  return agents.find((a) => a.tags?.includes('endurance')) ?? agents[0];
}

/** The group's agent as one row with « Changer », or the catalogue while changing it. */
function AgentField({
  agents,
  agent,
  changing,
  disabled,
  onChange,
  onSelect,
}: {
  agents: ReturnType<typeof useGroupAgents>;
  agent: Agent | null;
  changing: boolean;
  disabled: boolean;
  onChange: () => void;
  onSelect: (agentId: string) => void;
}) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const label = (
    <Text className="px-4 text-sm font-medium text-on-surface">{t('onboarding.groupAgentLabel')}</Text>
  );

  let body: React.ReactNode;
  if (agents.isLoading) {
    body = <Text className="mt-2 px-4 text-sm text-on-surface-variant">{t('discover.loadingAgents')}</Text>;
  } else if (agents.isError) {
    body = (
      <Text className="mt-2 px-4 text-sm text-error" accessibilityRole="alert">
        {t('app.failedLoadAgents')}
      </Text>
    );
  } else if (agents.list.length === 0 || agent === null) {
    body = <Text className="mt-2 px-4 text-sm text-on-surface-variant">{t('app.noAgentsAvailable')}</Text>;
  } else if (changing) {
    body = (
      <View className="mt-2" testID="onboarding-group-agents">
        {agents.list.map((option, index) => {
          const checked = option.id === agent.id;
          return (
            <Row
              key={option.id}
              title={option.title}
              subtitle={t(coachCategoryLabelKey(option.category))}
              trailing={checked ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined}
              showChevron={false}
              onPress={disabled ? undefined : () => onSelect(option.id)}
              accessibilityRole="radio"
              accessibilityState={{ selected: checked }}
              last={index === agents.list.length - 1}
              testID={`onboarding-group-agent-${option.id}`}
            />
          );
        })}
      </View>
    );
  } else {
    body = (
      <View className="mt-2" testID="onboarding-group-agent">
        <Row
          title={agent.title}
          subtitle={t(coachCategoryLabelKey(agent.category))}
          trailing={
            <Pressable
              onPress={onChange}
              disabled={disabled}
              accessibilityRole="button"
              accessibilityLabel={t('onboarding.groupAgentChangeAria')}
              hitSlop={8}
              className="min-h-11 justify-center"
              testID="onboarding-group-agent-change"
            >
              <Text className="text-base font-medium text-primary">{t('onboarding.groupAgentChange')}</Text>
            </Pressable>
          }
          showChevron={false}
          last
        />
      </View>
    );
  }

  return (
    <View className="mt-6">
      {label}
      {body}
    </View>
  );
}

/** The invite, one job: share its link from the phone. The QR waits behind a link; the seat card tells the truth. */
function InviteShare({
  group,
  link,
  athleteName,
  toCoachHome,
  onDone,
}: {
  group: CoachingGroup;
  link: string;
  athleteName: string;
  toCoachHome: boolean;
  onDone: () => void;
}) {
  const { t } = useTranslation();
  const [showQr, setShowQr] = useState(false);
  const intro = athleteName
    ? t('onboarding.groupInviteIntroNamed', { name: athleteName, group: group.name, days: ONBOARDING_INVITE_DAYS })
    : t('onboarding.groupInviteLinkIntro', { group: group.name, days: ONBOARDING_INVITE_DAYS });

  return (
    <>
      <Text className="mt-4 text-3xl font-display text-left text-on-surface">
        {athleteName ? t('onboarding.groupInviteHeadingNamed', { name: athleteName }) : t('onboarding.groupInviteHeading')}
      </Text>
      <Text className="mt-3 text-sm text-on-surface-variant">{intro}</Text>

      <View className="mt-8 rounded-lg bg-surface-container px-3 py-3">
        <Text className="font-mono text-sm text-on-surface" numberOfLines={1} selectable testID="onboarding-group-link">
          {link}
        </Text>
      </View>

      <View className="mt-4 gap-2">
        <Button
          title={t('onboarding.groupShareLink')}
          onPress={() => {
            void Share.share({
              message: t('groups.inviteShareMessage', { group: group.name, link }),
            }).catch(() => {});
          }}
          fullWidth
          testID="onboarding-group-share"
        />
        <Button
          title={showQr ? t('onboarding.groupHideQr') : t('onboarding.groupShowQr')}
          variant="ghost"
          onPress={() => setShowQr((open) => !open)}
          testID="onboarding-group-qr-toggle"
        />
      </View>

      {showQr ? (
        <View
          className="mt-4 self-center rounded-md p-3"
          style={{ backgroundColor: QR_GROUND }}
          accessibilityLabel={t('onboarding.groupInviteQrAlt')}
          testID="onboarding-group-qr"
        >
          <QRCode value={link} size={200} backgroundColor={QR_GROUND} color={QR_INK} />
        </View>
      ) : null}

      <SeatCard group={group} />

      <View className="mt-8">
        <Button
          title={
            toCoachHome
              ? t('humanCoach.seatInvite.openCoachHome')
              : athleteName
                ? t('onboarding.groupDoneNamed', { name: athleteName })
                : t('onboarding.groupDone')
          }
          variant="secondary"
          onPress={onDone}
          fullWidth
          testID="onboarding-group-done"
        />
      </View>
    </>
  );
}

/**
 * Where the coach's seat stands for this group, as the server has it: its
 * coach, a request sent on a date, or nothing asked yet with the quiet
 * request. "Pending" is never said before a request exists.
 */
function SeatCard({ group }: { group: CoachingGroup }) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const { user } = useAuth();
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
    enabled: !group.coach_user_id,
  });

  if (group.coach_user_id) {
    return (
      <View
        className="mt-8 flex-row items-start gap-3 rounded-xl border border-border-faint bg-surface-container-lowest px-4 py-3"
        testID="onboarding-group-seat-coach"
      >
        <Feather name="check-circle" size={20} color={colors.success} />
        <View className="flex-1">
          <Text className="text-base font-semibold text-on-surface">{t('humanCoach.inviteSeatCoach')}</Text>
          <Text className="mt-0.5 text-sm text-on-surface-variant">{t('humanCoach.inviteSeatCoachBody')}</Text>
        </View>
      </View>
    );
  }

  // The request half of the seat, read as the room strip and Group info read it.
  const request = latest.data?.request ?? null;
  const pending = request !== null && coachSeatState(request) === 'pending' ? request : null;
  // Who may ask is the room's rule: the coach made this group on the coach
  // path, so they own it; coach access leaves nothing to ask.
  const mayAsk = canRequestCoachAccess({
    group,
    isOwner: true,
    managesRoster: user?.manages_roster === true,
  });
  return (
    <View
      className="mt-8 rounded-xl border border-border-faint bg-surface-container-lowest px-4 py-3"
      testID={pending ? 'onboarding-group-seat-requested' : 'onboarding-group-seat-none'}
    >
      <Text className="text-base font-semibold text-on-surface">{t('humanCoach.inviteSeatPending')}</Text>
      {pending ? (
        <Text className="mt-1 text-sm text-on-surface-variant">
          {t('humanCoach.inviteSeatRequestedNotice', { date: coachSeatMoment(pending.created_at, language).date })}
        </Text>
      ) : latest.isLoading ? null : (
        <Text className="mt-1 text-sm text-on-surface-variant">{t('humanCoach.inviteSeatNotRequested')}</Text>
      )}
      <Text className="mt-1 text-sm text-on-surface-variant">{t('humanCoach.inviteSeatMeanwhile')}</Text>
      {pending || latest.isLoading || !mayAsk ? null : (
        <CoachAccessRequest groupId={group.id} />
      )}
    </View>
  );
}
