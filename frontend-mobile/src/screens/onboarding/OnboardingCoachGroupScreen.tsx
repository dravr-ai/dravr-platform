// ABOUTME: Onboarding step (mobile) — a coach names their group, picks the agent its athletes talk to, leaves with the invite
// ABOUTME: Mirrors the web OnboardingCoachGroup; coach access is never granted here (ADR-018), only requested in one tap

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, Share } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Feather } from '@expo/vector-icons';
import * as Clipboard from 'expo-clipboard';
import QRCode from 'react-native-qrcode-svg';
import type { CoachingGroup } from '@pierre/shared-types';
import { BOREAL_LIGHT, QUERY_KEYS, coachCategoryLabelKey, isCoachFacing } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button, Input, Row } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { useAuth } from '../../contexts/AuthContext';
import { useThemeColors } from '../../constants/theme';
import { chatApi, coachesApi, groupsApi, userApi } from '../../services/api';
import { COACH_GROUP_DONE_PREFIX, useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { inviteLink } from '../../constants/inviteLink';
import { CoachAccessRequest } from '../../components/CoachAccessRequest';

/**
 * How long the onboarding athlete invite stays valid — the web step's value:
 * a coach sets up during onboarding, often before the athlete is ready to
 * sign up, and the 7 days a chat-issued invite gets are easily missed.
 */
const ONBOARDING_INVITE_DAYS = 30;

/** A QR is not themed: the phone camera needs dark modules on a light ground in both schemes. */
const QR_GROUND = BOREAL_LIGHT.surfaceContainerLowest;
const QR_INK = BOREAL_LIGHT.onSurface;

/**
 * The coach's group step (mobile): name the group, pick its agent, then share
 * the athlete invite. The agent is chosen from the catalogue here rather than
 * inherited from the coach's own selection — a coach who does not train never
 * picked one. The server decides whether the coach is set as the group's
 * coach; a group that comes back without one is shown as access pending, with
 * a one-tap request a super-admin grants or declines (carnet#738).
 */
export function OnboardingCoachGroupScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const queryClient = useQueryClient();
  const { mark } = useOnboardingFlag(COACH_GROUP_DONE_PREFIX, user?.id);
  const progress = useOnboardingProgress('coach_group');
  const [name, setName] = useState('');
  const [pickingAgent, setPickingAgent] = useState(false);
  const [agentId, setAgentId] = useState<string | null>(null);
  const [created, setCreated] = useState<CoachingGroup | null>(null);
  const [link, setLink] = useState<string | null>(null);
  const [working, setWorking] = useState(false);
  const [failed, setFailed] = useState(false);
  const [copied, setCopied] = useState(false);

  const finish = async (status: 'complete' | 'skipped') => {
    userApi.setOnboardingStep('coach_group', status).catch(() => {});
    await mark();
  };

  const create = async () => {
    const trimmed = name.trim();
    if (!trimmed || !agentId || working) return;
    setWorking(true);
    setFailed(false);
    try {
      // A retry after a later call failed reuses the group already made.
      const group =
        created ??
        (await groupsApi.createGroup({ name: trimmed, agent_id: agentId, coach_is_me: true }));
      setCreated(group);
      await chatApi.createConversation({ group_id: group.id, agent_id: group.agent_id });
      // The chat list may already be cached from sign-in: the group's thread
      // must be on it when "Go to my group" leads there.
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
      const invite = await groupsApi.createInvite(group.id, {
        expires_in_days: ONBOARDING_INVITE_DAYS,
      });
      setLink(inviteLink(invite.code));
    } catch {
      setFailed(true);
    } finally {
      setWorking(false);
    }
  };

  if (created && link) {
    return (
      <SafeAreaView className="flex-1 bg-surface">
        <ScrollView contentContainerClassName="py-10 px-4">
          <OnboardingProgressBar steps={progress} />
          <Text className="mt-4 text-3xl font-display text-left text-on-surface">
            {t('onboarding.groupInviteHeading')}
          </Text>
          <Text className="mt-3 text-sm text-on-surface-variant">
            {t('onboarding.groupInviteIntro')}
          </Text>

          <View
            className="mt-8 self-center rounded-md p-3"
            style={{ backgroundColor: QR_GROUND }}
            accessibilityLabel={t('onboarding.groupInviteQrAlt')}
            testID="onboarding-group-qr"
          >
            <QRCode value={link} size={200} backgroundColor={QR_GROUND} color={QR_INK} />
          </View>

          <Text
            className="mt-6 text-center font-mono text-sm text-on-surface"
            selectable
            testID="onboarding-group-link"
          >
            {link}
          </Text>

          <View className="mt-6 gap-3">
            <Button
              title={copied ? t('onboarding.groupLinkCopied') : t('onboarding.groupCopyLink')}
              variant="secondary"
              onPress={() => {
                void Clipboard.setStringAsync(link).then(() => setCopied(true));
              }}
            />
            <Button
              title={t('onboarding.groupShare')}
              variant="secondary"
              onPress={() => {
                void Share.share({
                  message: t('groups.inviteShareMessage', { group: created.name, link }),
                }).catch(() => {});
              }}
            />
          </View>

          {created.coach_user_id ? null : (
            <View className="mt-8" testID="onboarding-group-access-pending">
              <Text className="text-lg font-display text-on-surface">
                {t('humanCoach.accessPendingHeading')}
              </Text>
              <Text className="mt-2 text-sm text-on-surface-variant">
                {t('humanCoach.accessPendingBody')}
              </Text>
              <CoachAccessRequest groupId={created.id} />
            </View>
          )}

          <View className="mt-8">
            <Button
              title={t('onboarding.groupDone')}
              onPress={() => void finish('complete')}
              testID="onboarding-group-done"
            />
          </View>
        </ScrollView>
      </SafeAreaView>
    );
  }

  if (pickingAgent) {
    return (
      <SafeAreaView className="flex-1 bg-surface">
        <ScrollView contentContainerClassName="py-10">
          <View className="px-4">
            <OnboardingProgressBar steps={progress} />
            <Text className="mt-4 text-3xl font-display text-left text-on-surface">
              {t('onboarding.groupAgentHeading')}
            </Text>
            <Text className="mt-3 text-sm text-on-surface-variant">
              {t('onboarding.groupAgentIntro')}
            </Text>
          </View>

          <AgentPicker
            selected={agentId}
            onSelect={(id) => {
              if (!working) setAgentId(id);
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
              onPress={() => void create()}
              disabled={working || agentId === null}
              testID="onboarding-group-create"
            />
            {/* Once the group exists its name and agent are fixed: going back
                would only offer edits the retry cannot apply. */}
            {created ? null : (
              <Pressable
                onPress={() => setPickingAgent(false)}
                disabled={working}
                accessibilityRole="button"
                testID="onboarding-group-back"
              >
                <Text className="text-center text-sm text-on-surface-variant">
                  {t('common.back')}
                </Text>
              </Pressable>
            )}
          </View>
        </ScrollView>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView className="flex-1 bg-surface">
      <ScrollView contentContainerClassName="py-10 px-4" keyboardShouldPersistTaps="handled">
        <OnboardingProgressBar steps={progress} />
        <Text className="mt-4 text-3xl font-display text-left text-on-surface">
          {t('onboarding.groupHeading')}
        </Text>
        <Text className="mt-3 text-sm text-on-surface-variant">{t('humanCoach.groupStepIntro')}</Text>

        <View className="mt-8">
          <Input
            label={t('onboarding.groupNameLabel')}
            placeholder={t('onboarding.groupNamePlaceholder')}
            value={name}
            maxLength={100}
            onChangeText={setName}
            testID="onboarding-group-name"
          />
        </View>

        <View className="mt-8 gap-3">
          <Button
            title={t('common.next')}
            onPress={() => setPickingAgent(true)}
            disabled={name.trim() === ''}
            testID="onboarding-group-next"
          />
          <Pressable
            onPress={() => void finish('skipped')}
            accessibilityRole="button"
            testID="onboarding-group-later"
          >
            <Text className="text-center text-sm text-on-surface-variant">
              {t('onboarding.groupLater')}
            </Text>
          </Pressable>
        </View>
      </ScrollView>
    </SafeAreaView>
  );
}

/**
 * The catalogue as a single-choice list. Unranked on purpose: the coach's own
 * activities are no evidence of what their athletes need. Coach-facing agents
 * are left out: the athletes talk to this one.
 */
function AgentPicker({
  selected,
  onSelect,
}: {
  selected: string | null;
  onSelect: (agentId: string) => void;
}) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const { data, isLoading, isError } = useQuery({
    queryKey: QUERY_KEYS.coaches.list(),
    queryFn: () => coachesApi.list(),
  });
  const agents = (data?.agents ?? []).filter((a) => !a.is_hidden && !isCoachFacing(a));

  if (isLoading) {
    return (
      <Text className="mt-8 px-4 text-center text-sm text-on-surface-variant">
        {t('discover.loadingAgents')}
      </Text>
    );
  }
  if (isError) {
    return (
      <Text className="mt-8 px-4 text-center text-sm text-error" accessibilityRole="alert">
        {t('app.failedLoadAgents')}
      </Text>
    );
  }
  if (agents.length === 0) {
    return (
      <Text className="mt-8 px-4 text-center text-sm text-on-surface-variant">
        {t('app.noAgentsAvailable')}
      </Text>
    );
  }

  return (
    <View className="mt-6" testID="onboarding-group-agents">
      {agents.map((agent, index) => {
        const checked = agent.id === selected;
        return (
          <Row
            key={agent.id}
            title={agent.title}
            subtitle={t(coachCategoryLabelKey(agent.category))}
            trailing={
              checked ? <Feather name="check" size={18} color={colors.tokens.primary} /> : undefined
            }
            showChevron={false}
            onPress={() => onSelect(agent.id)}
            accessibilityRole="radio"
            accessibilityState={{ selected: checked }}
            last={index === agents.length - 1}
            testID={`onboarding-group-agent-${agent.id}`}
          />
        );
      })}
    </View>
  );
}
