// ABOUTME: Onboarding step (mobile) — a coach creates their group and leaves with the athlete invite link and QR
// ABOUTME: Mirrors the web OnboardingCoachGroup; coach access is never granted here (ADR-018)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React, { useState } from 'react';
import { View, Text, ScrollView, Pressable, Share, Linking } from 'react-native';
import { SafeAreaView } from 'react-native-safe-area-context';
import * as Clipboard from 'expo-clipboard';
import QRCode from 'react-native-qrcode-svg';
import type { CoachingGroup } from '@pierre/shared-types';
import { BOREAL_LIGHT } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button, Input } from '../../components/ui';
import { OnboardingProgressBar } from '../../components/ui/OnboardingProgressBar';
import { useAuth } from '../../contexts/AuthContext';
import { chatApi, groupsApi, userApi } from '../../services/api';
import { COACH_GROUP_DONE_PREFIX, useOnboardingFlag } from '../../hooks/useOnboardingFlag';
import { useOnboardingProgress } from '../../hooks/useOnboardingProgress';
import { inviteLink } from '../../constants/inviteLink';

/**
 * How long the onboarding athlete invite stays valid — the web step's value:
 * a coach shares it with a whole roster, and five athletes rarely all join
 * within the 7 days a chat-issued invite gets.
 */
const ONBOARDING_INVITE_DAYS = 30;

/** A QR is not themed: the phone camera needs dark modules on a light ground in both schemes. */
const QR_GROUND = BOREAL_LIGHT.surfaceContainerLowest;
const QR_INK = BOREAL_LIGHT.onSurface;

/**
 * The coach's group step (mobile): name the group, then share the athlete
 * invite. The server decides whether the coach is set as the group's coach; a
 * group that comes back without one is shown as access pending.
 */
export function OnboardingCoachGroupScreen() {
  const { t } = useTranslation();
  const { user } = useAuth();
  const { mark } = useOnboardingFlag(COACH_GROUP_DONE_PREFIX, user?.id);
  const progress = useOnboardingProgress('coach_group');
  const [name, setName] = useState('');
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
    if (!trimmed || working) return;
    setWorking(true);
    setFailed(false);
    try {
      // A retry after a later call failed reuses the group already made.
      const group =
        created ?? (await groupsApi.createGroup({ name: trimmed, coach_is_me: true }));
      setCreated(group);
      await chatApi.createConversation({ group_id: group.id, agent_id: group.agent_id });
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
              <Pressable
                onPress={() => void Linking.openURL('mailto:support@dravr.ai').catch(() => {})}
                accessibilityRole="link"
              >
                <Text className="mt-3 text-sm font-medium text-primary">
                  {t('onboarding.groupAccessContact')}
                </Text>
              </Pressable>
            </View>
          )}

          <View className="mt-8">
            <Button title={t('onboarding.groupDone')} onPress={() => void finish('complete')} />
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
            error={failed ? t('onboarding.groupCreateFailed') : undefined}
            testID="onboarding-group-name"
          />
        </View>

        <View className="mt-8 gap-3">
          <Button
            title={working ? t('onboarding.groupCreating') : t('onboarding.groupCreate')}
            onPress={() => void create()}
            disabled={working || name.trim() === ''}
            testID="onboarding-group-create"
          />
          <Pressable
            onPress={() => void finish('skipped')}
            disabled={working}
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
