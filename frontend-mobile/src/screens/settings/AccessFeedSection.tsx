// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: "Who read your data" on the phone's privacy screen — outside apps' calls and agent turns, API keys', the coach's and group members' reads per day
// ABOUTME: A Connected app carries a Revoke that revokes every grant it holds; an API key or token points to its settings, as on web (carnet#1543)

import React, { useMemo } from 'react';
import { ActivityIndicator, Alert, StyleSheet, Text, View } from 'react-native';
import type { AccessFeedAppRead, AccessFeedCoachDay, AccessFeedPeerDay } from '@pierre/shared-types';
import { countKey, formatDate, formatDateTime } from '@pierre/chat-utils';
import {
  ACCESS_FEED_COACH_KEYS,
  ACCESS_FEED_DAY_READS_KEYS,
  ACCESS_FEED_DAY_VIEWS_KEYS,
  ACCESS_FEED_KEYS,
  ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS,
  ACCESS_FEED_PEER_DAY_READS_KEYS,
  ACCESS_FEED_PEER_DAY_VIEWS_KEYS,
  COACH_READ_LOG_KEYS,
  accessFeedCredentialKey,
  coachReadClassKey,
} from '@pierre/shared-constants';
import {
  describeApiError,
  groupAccessFeedApiKeyDays,
  groupAccessFeedApps,
  revokedInKeySettings,
  stepUpLines,
  type AccessFeedApiKeyGroup,
  type AccessFeedAppGroup,
} from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { useAccessFeed, useConnectedApps, useRevokeConnectedApps } from '../../hooks/useAccessFeed';

/**
 * One read an app made, and when: a tool it called, or a turn of Dravr's
 * agent it asked for, which chose its own tools and so names none.
 */
function AppReadRow({ read }: { read: AccessFeedAppRead }) {
  const { t, language } = useTranslation();
  const agentTurn = read.kind === 'agent_turn' || read.tool === null;
  return (
    <View className="py-1" testID={agentTurn ? 'access-feed-read-agent-turn' : `access-feed-read-${read.tool}`}>
      <Text className="text-sm text-text-secondary">
        {agentTurn ? t(ACCESS_FEED_KEYS.agentTurnRow) : t(ACCESS_FEED_KEYS.appRow, { tool: read.tool })}
      </Text>
      <Text className="text-xs text-text-tertiary">{formatDateTime(read.read_at, language)}</Text>
    </View>
  );
}

interface AppGroupProps {
  group: AccessFeedAppGroup;
  /** The last group draws no hairline under itself. */
  last: boolean;
  revoking: boolean;
  onRevoke: (group: AccessFeedAppGroup) => void;
}

/**
 * One outside app and its calls. Support's calls are one group that names no
 * client, since the client is the admin's. Revoke shows only for a Connected
 * app the athlete still holds a grant of.
 */
function AppGroup({ group, last, revoking, onRevoke }: AppGroupProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const credentialKey = accessFeedCredentialKey(group.credential);
  const name = group.name ?? t(ACCESS_FEED_KEYS.unknownApp);
  const meta = [
    group.host,
    credentialKey && t(credentialKey),
    group.transport === 'a2a' ? t(ACCESS_FEED_KEYS.overA2a) : null,
  ].filter(Boolean);
  return (
    <View
      className={last ? 'py-3' : 'py-3 border-b border-border-faint'}
      style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
      testID={group.kind === 'support' ? 'access-feed-support' : 'access-feed-app'}
    >
      <View className="flex-row items-start justify-between gap-3">
        <View className="flex-1">
          {group.kind === 'support' ? (
            <Text className="text-sm font-medium text-text-primary">{t(ACCESS_FEED_KEYS.supportRead)}</Text>
          ) : (
            <>
              <Text className="text-sm font-medium text-text-primary" testID="access-feed-app-name">
                {name}
              </Text>
              {meta.length > 0 && <Text className="text-xs text-text-secondary">{meta.join(' · ')}</Text>}
            </>
          )}
        </View>
        {group.revokeGrantIds.length > 0 &&
          (revoking ? (
            <ActivityIndicator size="small" color={colors.text.tertiary} />
          ) : (
            <Text
              className="text-sm font-medium text-primary"
              accessibilityRole="button"
              accessibilityLabel={t('app.revokeAppLabel', { app: name })}
              onPress={() => onRevoke(group)}
              testID="access-feed-revoke"
            >
              {t('app.revoke')}
            </Text>
          ))}
      </View>
      {group.stepUp && (
        <View className="mt-1" testID="access-feed-step-up">
          <Text className="text-xs text-text-secondary">
            {t('tokens.connectedAppsStepUpChild', {
              time: group.stepUp.expires_at ? formatDateTime(group.stepUp.expires_at, language) : '',
            })}
          </Text>
          {stepUpLines(group.stepUp, t).map((line) => (
            <Text key={line} className="text-xs text-text-secondary">
              {line}
            </Text>
          ))}
        </View>
      )}
      {group.kind === 'app' && revokedInKeySettings(group.credential) && (
        <Text className="text-xs text-text-secondary mt-1" testID="access-feed-key-hint">
          {t(ACCESS_FEED_KEYS.apiKeyHint)}
        </Text>
      )}
      <View className="mt-1">
        {group.reads.map((read, index) => (
          <AppReadRow key={`${read.read_at}-${read.tool}-${index}`} read={read} />
        ))}
      </View>
    </View>
  );
}

/**
 * One of the athlete's API keys and the days its REST requests read their
 * data. It is managed in the API-keys settings, so it carries the hint there
 * and no Revoke.
 */
function ApiKeyGroup({ group, last }: { group: AccessFeedApiKeyGroup; last: boolean }) {
  const { t, language } = useTranslation();
  return (
    <View
      className={last ? 'py-3' : 'py-3 border-b border-border-faint'}
      style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
      testID="access-feed-api-key"
    >
      <Text className="text-sm font-medium text-text-primary" testID="access-feed-api-key-name">
        {group.name}
      </Text>
      <Text className="text-xs text-text-secondary">
        {group.prefix} · {t(accessFeedCredentialKey('api_key'))}
      </Text>
      <Text className="text-xs text-text-secondary mt-1" testID="access-feed-key-hint">
        {t(ACCESS_FEED_KEYS.apiKeyHint)}
      </Text>
      <View className="mt-1">
        {group.days.map((day) => (
          <View key={day.day} className="py-1" testID="access-feed-api-key-day">
            <Text className="text-sm text-text-secondary" testID="access-feed-api-key-requests">
              {t(countKey(day.requests, language, ACCESS_FEED_API_KEY_DAY_REQUESTS_KEYS), { count: day.requests })}
            </Text>
            <Text className="text-xs text-text-tertiary">{formatDate(day.day, language, 'UTC')}</Text>
          </View>
        ))}
      </View>
    </View>
  );
}

/**
 * One day of the coach's reads in one group: who, which kind of data, how
 * many reads and group views, and whether an AI tool the coach connects got
 * them or Dravr support made them.
 */
function CoachDayRow({ day }: { day: AccessFeedCoachDay }) {
  const { t, language } = useTranslation();
  const group = day.group_name ?? t(ACCESS_FEED_COACH_KEYS.unknownGroup);
  const counts = [
    day.reads > 0 ? t(countKey(day.reads, language, ACCESS_FEED_DAY_READS_KEYS), { count: day.reads, group }) : null,
    day.views > 0 ? t(countKey(day.views, language, ACCESS_FEED_DAY_VIEWS_KEYS), { count: day.views, group }) : null,
  ].filter(Boolean);
  return (
    <View className="py-1.5" testID="access-feed-coach-day">
      <Text className="text-sm text-text-primary">
        <Text className="font-medium" testID="access-feed-coach-name">
          {day.coach_name ?? t(COACH_READ_LOG_KEYS.formerCoach)}
        </Text>
        {' · '}
        <Text>{t(coachReadClassKey(day.data_class))}</Text>
      </Text>
      <Text className="text-xs text-text-secondary mt-0.5" testID="access-feed-coach-counts">
        {counts.join(' · ')}
      </Text>
      <Text className="text-xs text-text-tertiary mt-0.5">{formatDate(day.day, language, 'UTC')}</Text>
      {day.support_read && (
        <Text className="text-xs text-text-secondary mt-0.5" testID="access-feed-coach-support">
          {t(COACH_READ_LOG_KEYS.supportRead)}
        </Text>
      )}
      {day.served_to_assistant && !day.support_read && (
        <Text className="text-xs text-text-secondary mt-0.5" testID="access-feed-coach-served">
          {t(COACH_READ_LOG_KEYS.servedToAssistant)}
        </Text>
      )}
    </View>
  );
}

/**
 * One day of a fellow group member's reads in one group, under the athlete's
 * peer sharing: the sentence naming who read how often when the member's
 * agent read, else the member alone, then the day's group views, and
 * whether an AI tool the member connects got them or Dravr support made them.
 */
function PeerDayRow({ day }: { day: AccessFeedPeerDay }) {
  const { t, language } = useTranslation();
  const member = day.member_name ?? t(ACCESS_FEED_KEYS.formerMember);
  const group = day.group_name ?? t(ACCESS_FEED_COACH_KEYS.unknownGroup);
  const line =
    day.reads > 0
      ? t(countKey(day.reads, language, ACCESS_FEED_PEER_DAY_READS_KEYS), { member, count: day.reads, group })
      : member;
  return (
    <View className="py-1.5" testID="access-feed-peer-day">
      <Text className="text-sm text-text-primary" testID="access-feed-peer-line">
        {line}
      </Text>
      {day.views > 0 && (
        <Text className="text-xs text-text-secondary mt-0.5" testID="access-feed-peer-views">
          {t(countKey(day.views, language, ACCESS_FEED_PEER_DAY_VIEWS_KEYS), { count: day.views, group })}
        </Text>
      )}
      <Text className="text-xs text-text-tertiary mt-0.5">{formatDate(day.day, language, 'UTC')}</Text>
      {day.support_read && (
        <Text className="text-xs text-text-secondary mt-0.5" testID="access-feed-peer-support">
          {t(ACCESS_FEED_KEYS.peerSupportRead)}
        </Text>
      )}
      {day.served_to_assistant && !day.support_read && (
        <Text className="text-xs text-text-secondary mt-0.5" testID="access-feed-peer-served">
          {t(ACCESS_FEED_KEYS.peerServedToAssistant)}
        </Text>
      )}
    </View>
  );
}

/**
 * The athlete's record of who read their data (carnet#1543): each call or
 * agent turn an outside app made over their own data, their API keys' days,
 * and their human coach's and fellow members' reads as daily counts, over the
 * window the server names.
 */
export function AccessFeedSection(): React.JSX.Element {
  const { t } = useTranslation();
  const { data: feed, isLoading, isError } = useAccessFeed();
  const { data: grants } = useConnectedApps();
  const revoke = useRevokeConnectedApps();

  const groups = useMemo(() => groupAccessFeedApps(feed?.app_reads ?? [], grants ?? []), [feed, grants]);
  const apiKeys = useMemo(() => groupAccessFeedApiKeyDays(feed?.api_key_days ?? []), [feed]);
  const coachDays = feed?.coach_days ?? [];
  const peerDays = feed?.peer_days ?? [];
  const nothingRead = groups.length === 0 && apiKeys.length === 0 && coachDays.length === 0 && peerDays.length === 0;

  const handleRevoke = (group: AccessFeedAppGroup): void => {
    const app = group.name ?? t(ACCESS_FEED_KEYS.unknownApp);
    Alert.alert(t('app.revokeAccessQ'), t('app.confirmRevokeAppAccess', { app }), [
      { text: t('common.cancel'), style: 'cancel' },
      {
        text: t('app.revoke'),
        style: 'destructive',
        onPress: () =>
          revoke.mutate(group.revokeGrantIds, {
            onError: (err: unknown) => {
              Alert.alert(t('app.couldNotRevokeApp'), describeApiError(err, { t, fallbackKey: 'errors.unknown' }));
            },
          }),
      },
    ]);
  };

  return (
    <Section
      title={t(ACCESS_FEED_KEYS.title)}
      description={feed ? t(ACCESS_FEED_KEYS.intro, { days: feed.days }) : undefined}
      testID="privacy-section-access-feed"
    >
      {isError ? (
        <Text className="px-4 text-sm text-error" testID="access-feed-error">
          {t(ACCESS_FEED_KEYS.loadFailed)}
        </Text>
      ) : isLoading || !feed ? null : nothingRead ? (
        <EmptyState testID="access-feed-empty">{t(ACCESS_FEED_KEYS.empty, { days: feed.days })}</EmptyState>
      ) : (
        <View className="gap-6 px-4">
          {(groups.length > 0 || apiKeys.length > 0) && (
            <View testID="access-feed-apps">
              <Text className="text-sm font-medium text-text-primary">{t(ACCESS_FEED_KEYS.appsHeading)}</Text>
              {groups.map((group, index) => (
                <AppGroup
                  key={group.key}
                  group={group}
                  last={index === groups.length - 1 && apiKeys.length === 0}
                  revoking={revoke.isPending && (revoke.variables?.some((id) => group.revokeGrantIds.includes(id)) ?? false)}
                  onRevoke={handleRevoke}
                />
              ))}
              {apiKeys.map((key, index) => (
                <ApiKeyGroup key={key.keyId} group={key} last={index === apiKeys.length - 1} />
              ))}
            </View>
          )}
          {coachDays.length > 0 && (
            <View testID="access-feed-coach-days">
              <Text className="text-sm font-medium text-text-primary">{t(ACCESS_FEED_COACH_KEYS.feedHeading)}</Text>
              {coachDays.map((day, index) => (
                <CoachDayRow key={`${day.day}-${day.group_id}-${day.data_class}-${day.support_read}-${index}`} day={day} />
              ))}
            </View>
          )}
          {peerDays.length > 0 && (
            <View testID="access-feed-peer-days">
              <Text className="text-sm font-medium text-text-primary">{t(ACCESS_FEED_KEYS.peersHeading)}</Text>
              {peerDays.map((day, index) => (
                <PeerDayRow key={`${day.day}-${day.group_id}-${day.member_name ?? ''}-${index}`} day={day} />
              ))}
            </View>
          )}
        </View>
      )}
    </Section>
  );
}
