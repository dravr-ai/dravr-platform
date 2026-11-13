// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access request (mobile) for a coachless group the coach owns — inline in onboarding, a strip over the room, a block in Group info
// ABOUTME: Mirrors the web CoachAccessRequest: the latest request's state from the server, and asking grants nothing (ADR-018)

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  COACH_PLATFORM_PROVIDERS,
  COACH_SEAT_KEYS,
  QUERY_KEYS,
  coachPlatformName,
  coachSeatMoment,
  coachSeatState,
  type CoachSeatState,
} from '@pierre/shared-constants';
import type { CoachAccessRequest as CoachAccessRequestRecord } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { Button } from './ui';
import { ProviderGlyph } from './ProviderGlyph';
import { userApi } from '../services/api';
import { useThemeColors } from '../constants/theme';
import { useProviderConnected } from '../hooks/useHome';

/**
 * Where the request is shown — the same three places as the web:
 * `inline` under the onboarding group step's copy, `strip` at the top of the
 * group room, `block` first in Group info.
 */
export type CoachAccessRequestVariant = 'inline' | 'strip' | 'block';

interface CoachAccessRequestProps {
  groupId: string;
  variant?: CoachAccessRequestVariant;
  /** `strip` only: open Group info, where the coaching platforms are offered. */
  onOpenInfo?: () => void;
  /**
   * `block` only: open Connections, where each coaching platform's own
   * connect flow runs (its sign-in or its key form).
   */
  onOpenConnections?: () => void;
}

/**
 * The one coach-access request for a coachless group (carnet#738), behind
 * every entry point so none can drift, mirroring the web. The request names
 * this group so a grant attaches the coach to it; asking grants nothing
 * (ADR-018), and the pending state promises no notification the product does
 * not send.
 */
export function CoachAccessRequest({ groupId, variant = 'inline', onOpenInfo, onOpenConnections }: CoachAccessRequestProps) {
  const queryClient = useQueryClient();
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const ask = useMutation({
    mutationFn: () => userApi.requestCoachAccess(groupId),
    onSuccess: (data) => {
      queryClient.setQueryData(QUERY_KEYS.user.coachAccessRequest(), data);
    },
  });

  const request = latest.data?.request ?? null;

  if (variant === 'inline') {
    return <InlineRequest status={request?.status} ask={ask} loading={latest.isLoading} />;
  }

  // Nothing until the read answers, so the seat never flashes "to ask" before
  // a request already waiting. A failed read offers the request: the server
  // keeps one pending request per account.
  if (latest.isLoading) return null;

  const view: SeatViewProps = {
    state: coachSeatState(request),
    request,
    asking: ask.isPending,
    failed: ask.isError,
    onAsk: () => ask.mutate(),
  };
  return variant === 'strip' ? (
    <SeatStrip {...view} onOpenInfo={onOpenInfo} />
  ) : (
    <SeatBlock {...view} onOpenConnections={onOpenConnections} />
  );
}

/** The onboarding step's request: the button, a declined line above it, or the pending sentence. */
function InlineRequest({
  status,
  ask,
  loading,
}: {
  status: CoachAccessRequestRecord['status'] | undefined;
  ask: { mutate: () => void; isPending: boolean; isError: boolean };
  loading: boolean;
}) {
  const { t } = useTranslation();
  if (status === 'pending') {
    return (
      <Text className="mt-3 text-sm text-on-surface" testID="coach-access-pending">
        {t('humanCoach.requestAccessPending')}
      </Text>
    );
  }

  return (
    <View className="mt-3 gap-2">
      {status === 'declined' ? (
        <Text className="text-sm text-on-surface-variant" testID="coach-access-declined">
          {t('humanCoach.requestAccessDeclined')}
        </Text>
      ) : null}
      <Button
        title={ask.isPending ? t('humanCoach.requestAccessSending') : t('humanCoach.requestAccess')}
        variant="secondary"
        onPress={() => ask.mutate()}
        disabled={ask.isPending || loading}
        testID="coach-access-request"
      />
      {ask.isError ? (
        <Text className="text-sm text-error" accessibilityRole="alert" testID="coach-access-failed">
          {t('humanCoach.requestAccessFailed')}
        </Text>
      ) : null}
    </View>
  );
}

interface SeatViewProps {
  state: CoachSeatState;
  request: CoachAccessRequestRecord | null;
  asking: boolean;
  failed: boolean;
  onAsk: () => void;
}

/** The filled request, full width: "Request coach access", or "Ask again" after a decline. */
function AskButton({ state, asking, onAsk }: Pick<SeatViewProps, 'state' | 'asking' | 'onAsk'>) {
  const { t } = useTranslation();
  return (
    <Button
      title={asking ? t(COACH_SEAT_KEYS.sending) : state === 'declined' ? t(COACH_SEAT_KEYS.askAgain) : t(COACH_SEAT_KEYS.request)}
      onPress={onAsk}
      disabled={asking}
      fullWidth
      testID="coach-access-request"
    />
  );
}

function AskFailed() {
  const { t } = useTranslation();
  return (
    <Text className="text-sm text-error" accessibilityRole="alert" testID="coach-access-failed">
      {t(COACH_SEAT_KEYS.failed)}
    </Text>
  );
}

/**
 * The room's top strip: the coach sees, without opening anything, that they
 * are not this group's coach yet and the one filled way to ask; once asked,
 * the date, who reviews it and how long that usually takes.
 */
function SeatStrip({ state, request, asking, failed, onAsk, onOpenInfo }: SeatViewProps & { onOpenInfo?: () => void }) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const pending = state === 'pending';
  const sent = request ? coachSeatMoment(request.created_at, language) : null;
  const decided = request?.decided_at ? coachSeatMoment(request.decided_at, language) : sent;
  const title =
    pending && sent
      ? t(COACH_SEAT_KEYS.pendingTitle, { date: sent.date })
      : state === 'declined'
        ? t(COACH_SEAT_KEYS.declinedTitle)
        : t(COACH_SEAT_KEYS.title);

  return (
    <View className="mx-4 mt-3 rounded-xl border border-border-faint bg-surface-container-lowest p-4 gap-1.5" testID="coach-seat-strip">
      <View className="flex-row items-center gap-3">
        <View
          className={`w-9 h-9 rounded-full items-center justify-center ${pending ? 'bg-warning/15' : 'bg-primary-container'}`}
        >
          <Feather
            name={pending ? 'clock' : 'user-check'}
            size={18}
            color={pending ? colors.ink.warning : colors.tokens.onPrimaryContainer}
          />
        </View>
        <Text
          className="flex-1 text-base font-semibold text-text-primary"
          accessibilityRole="header"
          testID={`coach-seat-strip-${state}`}
        >
          {title}
        </Text>
      </View>
      {pending ? (
        <Text className="text-sm text-text-secondary" testID="coach-access-pending">
          {t(COACH_SEAT_KEYS.pendingBody)}
        </Text>
      ) : state === 'declined' && decided ? (
        <Text className="text-sm text-text-secondary" testID="coach-access-declined">
          {t(COACH_SEAT_KEYS.declinedBody, { date: decided.date })}
        </Text>
      ) : (
        <Text className="text-sm text-text-secondary">{t(COACH_SEAT_KEYS.stripBody)}</Text>
      )}
      {!pending && (
        <View className="mt-2">
          <AskButton state={state} asking={asking} onAsk={onAsk} />
          {onOpenInfo && (
            <Button
              title={t(COACH_SEAT_KEYS.linkAccount)}
              variant="ghost"
              onPress={onOpenInfo}
              fullWidth
              testID="coach-seat-open-info"
            />
          )}
        </View>
      )}
      {failed && <AskFailed />}
    </View>
  );
}

/**
 * Group info's first block: the state as its title, what is blocked, the
 * sheet's one filled request, then TrainingPeaks and Intervals.icu as plain
 * identical rows; once asked, the dated three-step timeline.
 */
function SeatBlock({ state, request, asking, failed, onAsk, onOpenConnections }: SeatViewProps & { onOpenConnections?: () => void }) {
  const { t, language } = useTranslation();
  const pending = state === 'pending';
  const decided = request ? coachSeatMoment(request.decided_at ?? request.created_at, language) : null;

  return (
    <View className="mt-4 rounded-xl border border-border-faint bg-surface-container-lowest p-4 gap-2" testID="coach-seat-block">
      <Text className="text-lg font-semibold text-text-primary" accessibilityRole="header" testID={`coach-seat-block-${state}`}>
        {pending
          ? t(COACH_SEAT_KEYS.pendingBlockTitle)
          : state === 'declined'
            ? t(COACH_SEAT_KEYS.declinedTitle)
            : t(COACH_SEAT_KEYS.title)}
      </Text>
      {pending && request ? (
        <SeatTimeline request={request} />
      ) : (
        <>
          {state === 'declined' && decided ? (
            <Text className="text-sm text-text-secondary" testID="coach-access-declined">
              {t(COACH_SEAT_KEYS.declinedBody, { date: decided.date })}
            </Text>
          ) : (
            <Text className="text-sm text-text-secondary">{t(COACH_SEAT_KEYS.blockBody)}</Text>
          )}
          <View className="mt-1">
            <AskButton state={state} asking={asking} onAsk={onAsk} />
          </View>
          {failed && <AskFailed />}
          <CoachPlatformRows onOpenConnections={onOpenConnections} />
        </>
      )}
    </View>
  );
}

/**
 * TrainingPeaks and Intervals.icu as identical rows: glyph, name, Connect.
 * A tap opens Connections, where the platform's own flow runs — what the
 * server does with the account shows here afterwards, read from the request.
 */
function CoachPlatformRows({ onOpenConnections }: { onOpenConnections?: () => void }) {
  const { t } = useTranslation();
  const { providers } = useProviderConnected();
  return (
    <View className="mt-3 pt-3 border-t border-border" testID="coach-seat-routes">
      <Text className="text-xs text-text-tertiary">{t(COACH_SEAT_KEYS.routesLabel)}</Text>
      {COACH_PLATFORM_PROVIDERS.map((id, index) => {
        const row = providers?.find((p) => p.provider === id);
        const name = row?.display_name ?? coachPlatformName(id);
        const connected = row?.connected === true;
        return (
          <TouchableOpacity
            key={id}
            onPress={onOpenConnections}
            disabled={connected || !onOpenConnections}
            accessibilityRole="button"
            accessibilityLabel={connected ? t('providers.isConnectedAria', { provider: name }) : t('providers.connectToAria', { provider: name })}
            className={`flex-row items-center gap-3 min-h-11 ${index > 0 ? 'border-t border-border-faint' : ''}`}
            testID={`coach-seat-route-${id}`}
          >
            <ProviderGlyph providerId={id} label={name} size={20} />
            <Text className="flex-1 text-sm font-medium text-text-primary">{name}</Text>
            <Text className={`text-sm font-medium ${connected ? 'text-text-secondary' : 'text-primary'}`}>
              {connected ? t('providers.connected') : t('providers.connect')}
            </Text>
          </TouchableOpacity>
        );
      })}
    </View>
  );
}

/**
 * The pending request as three steps (P1·C): sent, with its date and time;
 * the Dravr team reviewing, usually within 24 hours; then the seat and what
 * it opens. No notification is promised.
 */
function SeatTimeline({ request }: { request: CoachAccessRequestRecord }) {
  const { t, language } = useTranslation();
  const sent = coachSeatMoment(request.created_at, language);
  return (
    <View accessibilityLabel={t(COACH_SEAT_KEYS.timelineLabel)} className="pt-1" testID="coach-access-pending">
      <TimelineStep
        marker="done"
        title={t(COACH_SEAT_KEYS.stepSent)}
        status={t(COACH_SEAT_KEYS.stepDone)}
        detail={t(COACH_SEAT_KEYS.stepSentWhen, { date: sent.date, time: sent.time })}
        testID="coach-seat-step-sent"
      />
      <TimelineStep
        marker="current"
        title={t(COACH_SEAT_KEYS.stepReview)}
        status={t(COACH_SEAT_KEYS.stepNow)}
        detail={t(COACH_SEAT_KEYS.stepReviewWhen)}
        testID="coach-seat-step-review"
      />
      <TimelineStep
        marker="upcoming"
        title={t(COACH_SEAT_KEYS.stepSeat)}
        detail={t(COACH_SEAT_KEYS.stepSeatWhat)}
        last
        testID="coach-seat-step-seat"
      />
    </View>
  );
}

function TimelineStep({
  marker,
  title,
  status,
  detail,
  last = false,
  testID,
}: {
  marker: 'done' | 'current' | 'upcoming';
  title: string;
  status?: string;
  detail: string;
  last?: boolean;
  testID: string;
}) {
  const colors = useThemeColors();
  return (
    <View className="flex-row gap-3" testID={testID}>
      <View className="items-center">
        {marker === 'done' ? (
          <View className="w-6 h-6 rounded-full bg-primary items-center justify-center">
            <Feather name="check" size={14} color={colors.tokens.onPrimary} />
          </View>
        ) : marker === 'current' ? (
          <View className="w-6 h-6 rounded-full border-2 border-primary items-center justify-center">
            <View className="w-2 h-2 rounded-full bg-primary" />
          </View>
        ) : (
          <View className="w-6 h-6 rounded-full border-2 border-outline" />
        )}
        {!last && <View className={`w-0.5 flex-1 min-h-4 ${marker === 'done' ? 'bg-primary' : 'bg-outline-variant'}`} />}
      </View>
      <View className={`flex-1 pt-0.5 ${last ? 'pb-1' : 'pb-4'}`}>
        <Text className="text-sm font-semibold text-text-primary">
          {title}
          {status ? <Text className="font-normal text-text-secondary">{` · ${status}`}</Text> : null}
        </Text>
        <Text className="text-sm text-text-secondary">{detail}</Text>
      </View>
    </View>
  );
}
