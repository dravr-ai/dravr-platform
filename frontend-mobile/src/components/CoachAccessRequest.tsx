// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access request (mobile) — inline in onboarding, a strip over the room, a block in Group info, a panel in Settings and on Home
// ABOUTME: Mirrors the web CoachAccessRequest: to ask, pending + cancel, declined + reason, granted, from the server; asking grants nothing (ADR-018)

import React from 'react';
import { View, Text, TouchableOpacity } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  COACH_PLATFORM_PROVIDERS,
  COACH_SEAT_KEYS,
  QUERY_KEYS,
  coachPlatformName,
  coachSeatDeclineReason,
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
 * Where the request is shown — the same places as the web: `inline` under the
 * onboarding group step's copy, `strip` at the top of the group room, `block`
 * first in Group info, and `panel` in Settings and on Home for a coach with no
 * group (carnet#1522), which says the access is on once it is.
 */
export type CoachAccessRequestVariant = 'inline' | 'strip' | 'block' | 'panel';

interface CoachAccessRequestProps {
  /** The group the coach asks from; none in Settings or on Home. */
  groupId?: string;
  variant?: CoachAccessRequestVariant;
  /** `panel` only: the viewer holds coach access, so the panel says it is on. */
  managesRoster?: boolean;
  /** `panel` only: open the coach's Home, once the access is on. */
  onOpenHome?: () => void;
  /** `strip` only: open Group info, where the coaching platforms are offered. */
  onOpenInfo?: () => void;
  /**
   * `block` only: open Connections, where each coaching platform's own
   * connect flow runs (its sign-in or its key form).
   */
  onOpenConnections?: () => void;
}

/**
 * The one coach-access request (carnet#738), behind every entry point so none
 * can drift, mirroring the web. A request asked from a group names it; either
 * way a grant attaches the coach to every coachless group they own
 * (carnet#1521). Asking grants nothing (ADR-018): the pending state dates the
 * request, says the answer comes here and by email, and offers to withdraw it
 * (carnet#1522); a declined request shows the team's reason beside "Ask again".
 */
export function CoachAccessRequest({
  groupId,
  variant = 'inline',
  onOpenInfo,
  onOpenConnections,
  managesRoster = false,
  onOpenHome,
}: CoachAccessRequestProps) {
  const queryClient = useQueryClient();
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const cancel = useMutation({
    mutationFn: () => userApi.cancelCoachAccessRequest(),
    onSuccess: (data) => {
      queryClient.setQueryData(QUERY_KEYS.user.coachAccessRequest(), data);
    },
    // A decision that won the race, or a request already gone: read it again
    // so the state shown is the server's.
    onError: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.user.coachAccessRequest() });
    },
  });
  const ask = useMutation({
    mutationFn: () => userApi.requestCoachAccess(groupId),
    onMutate: () => cancel.reset(),
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
    cancelling: cancel.isPending,
    cancelFailed: cancel.isError,
    justCancelled: cancel.isSuccess,
    onCancel: () => cancel.mutate(),
  };
  if (variant === 'panel') {
    return managesRoster ? <SeatGranted onOpenHome={onOpenHome} /> : <SeatPanel {...view} />;
  }
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
  /** The waiting request is being withdrawn. */
  cancelling: boolean;
  /** Withdrawing it failed. */
  cancelFailed: boolean;
  /** It was just withdrawn from this view: say so where it stood. */
  justCancelled: boolean;
  onCancel: () => void;
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
 * Withdraw the waiting request — the quiet 44 pt action under the pending
 * state, never a second filled button. The request is deleted, so the coach
 * may ask again at once (carnet#1522).
 */
function CancelButton({ cancelling, onCancel }: Pick<SeatViewProps, 'cancelling' | 'onCancel'>) {
  const { t } = useTranslation();
  return (
    <Button
      title={cancelling ? t(COACH_SEAT_KEYS.cancelling) : t(COACH_SEAT_KEYS.cancel)}
      variant="ghost"
      onPress={onCancel}
      disabled={cancelling}
      fullWidth
      testID="coach-access-cancel"
    />
  );
}

/** Where a withdrawal left things: done, and asking again is open; or it failed. */
function CancelOutcome({ justCancelled, cancelFailed }: Pick<SeatViewProps, 'justCancelled' | 'cancelFailed'>) {
  const { t } = useTranslation();
  if (cancelFailed) {
    return (
      <Text className="text-sm text-error" accessibilityRole="alert" testID="coach-access-cancel-failed">
        {t(COACH_SEAT_KEYS.cancelFailed)}
      </Text>
    );
  }
  if (justCancelled) {
    return (
      <Text className="text-sm text-text-secondary" accessibilityLiveRegion="polite" testID="coach-access-cancelled">
        {t(COACH_SEAT_KEYS.cancelled)}
      </Text>
    );
  }
  return null;
}

/** The pending sentence: the day it was sent, and that the answer comes here and by email. */
function PendingSentence({ request }: { request: CoachAccessRequestRecord }) {
  const { t, language } = useTranslation();
  const sent = coachSeatMoment(request.created_at, language);
  return (
    <Text className="text-sm text-text-secondary" testID="coach-access-pending">
      {t(COACH_SEAT_KEYS.pendingSentBody, { date: sent.date })}
    </Text>
  );
}

/** A declined request: the day, "you can ask again", and the team's reason when it gave one. */
function DeclinedLines({ request }: { request: CoachAccessRequestRecord }) {
  const { t, language } = useTranslation();
  const decided = coachSeatMoment(request.decided_at ?? request.created_at, language);
  const reason = coachSeatDeclineReason(request);
  return (
    <>
      <Text className="text-sm text-text-secondary" testID="coach-access-declined">
        {t(COACH_SEAT_KEYS.declinedBody, { date: decided.date })}
      </Text>
      {reason ? (
        <Text className="text-sm text-text-primary" testID="coach-access-decline-reason">
          {t(COACH_SEAT_KEYS.declinedReason, { reason })}
        </Text>
      ) : null}
    </>
  );
}

/** A round glyph beside a title: the seat to take, the clock while the team reviews, the check once it is on. */
function SeatGlyph({ kind }: { kind: 'ask' | 'pending' | 'granted' }) {
  const colors = useThemeColors();
  const pending = kind === 'pending';
  return (
    <View
      className={`w-9 h-9 rounded-full items-center justify-center ${pending ? 'bg-warning/15' : 'bg-primary-container'}`}
    >
      <Feather
        name={pending ? 'clock' : kind === 'granted' ? 'check-circle' : 'user-check'}
        size={18}
        color={pending ? colors.ink.warning : colors.tokens.onPrimaryContainer}
      />
    </View>
  );
}

/**
 * The room's top strip: the coach sees, without opening anything, that they
 * are not this group's coach yet and the one filled way to ask; once asked,
 * the date, that the answer comes here and by email — no wait time is
 * promised — and the way to withdraw it.
 */
function SeatStrip({
  state,
  request,
  asking,
  failed,
  onAsk,
  onOpenInfo,
  cancelling,
  cancelFailed,
  justCancelled,
  onCancel,
}: SeatViewProps & { onOpenInfo?: () => void }) {
  const { t } = useTranslation();
  const pending = state === 'pending';
  const title = pending
    ? t(COACH_SEAT_KEYS.pendingBlockTitle)
    : state === 'declined'
      ? t(COACH_SEAT_KEYS.declinedTitle)
      : t(COACH_SEAT_KEYS.title);

  return (
    <View className="mx-4 mt-3 rounded-xl border border-border-faint bg-surface-container-lowest p-4 gap-1.5" testID="coach-seat-strip">
      <View className="flex-row items-center gap-3">
        <SeatGlyph kind={pending ? 'pending' : 'ask'} />
        <Text
          className="flex-1 text-base font-semibold text-text-primary"
          accessibilityRole="header"
          testID={`coach-seat-strip-${state}`}
        >
          {title}
        </Text>
      </View>
      {pending && request ? (
        <>
          <PendingSentence request={request} />
          <CancelButton cancelling={cancelling} onCancel={onCancel} />
        </>
      ) : state === 'declined' && request ? (
        <DeclinedLines request={request} />
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
      <CancelOutcome justCancelled={justCancelled && !pending} cancelFailed={cancelFailed} />
    </View>
  );
}

/**
 * Group info's first block: the state as its title, what is blocked, the
 * sheet's one filled request, then TrainingPeaks and Intervals.icu as plain
 * identical rows; once asked, the dated three-step timeline.
 */
function SeatBlock({
  state,
  request,
  asking,
  failed,
  onAsk,
  onOpenConnections,
  cancelling,
  cancelFailed,
  justCancelled,
  onCancel,
}: SeatViewProps & { onOpenConnections?: () => void }) {
  const { t } = useTranslation();
  const pending = state === 'pending';

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
        <>
          <SeatTimeline request={request} />
          <CancelButton cancelling={cancelling} onCancel={onCancel} />
          <CancelOutcome justCancelled={false} cancelFailed={cancelFailed} />
        </>
      ) : (
        <>
          {state === 'declined' && request ? (
            <DeclinedLines request={request} />
          ) : (
            <Text className="text-sm text-text-secondary">{t(COACH_SEAT_KEYS.blockBody)}</Text>
          )}
          <View className="mt-1">
            <AskButton state={state} asking={asking} onAsk={onAsk} />
          </View>
          {failed && <AskFailed />}
          <CancelOutcome justCancelled={justCancelled} cancelFailed={false} />
          <CoachPlatformRows onOpenConnections={onOpenConnections} />
        </>
      )}
    </View>
  );
}

/**
 * The group-less panel, in Settings and on Home (carnet#1522): what coach
 * access opens and the one filled request; once asked, the dated sentence and
 * the way to withdraw it; after a decline, the reason and "Ask again". The
 * request names no group: a grant attaches the coach to every coachless group
 * they own, and the groups they make afterwards are coached from the start.
 */
function SeatPanel({
  state,
  request,
  asking,
  failed,
  onAsk,
  cancelling,
  cancelFailed,
  justCancelled,
  onCancel,
}: SeatViewProps) {
  const { t } = useTranslation();
  const pending = state === 'pending';
  return (
    <View
      className="mx-4 rounded-xl border border-border-faint bg-surface-container-lowest p-4 gap-1.5"
      testID="coach-seat-panel"
    >
      <View className="flex-row items-center gap-3">
        <SeatGlyph kind={pending ? 'pending' : 'ask'} />
        <Text
          className="flex-1 text-base font-semibold text-text-primary"
          accessibilityRole="header"
          testID={`coach-seat-panel-${state}`}
        >
          {pending
            ? t(COACH_SEAT_KEYS.pendingBlockTitle)
            : state === 'declined'
              ? t(COACH_SEAT_KEYS.declinedTitle)
              : t(COACH_SEAT_KEYS.panelTitle)}
        </Text>
      </View>
      {pending && request ? (
        <>
          <PendingSentence request={request} />
          <CancelButton cancelling={cancelling} onCancel={onCancel} />
        </>
      ) : (
        <>
          {state === 'declined' && request ? (
            <DeclinedLines request={request} />
          ) : (
            <Text className="text-sm text-text-secondary">{t(COACH_SEAT_KEYS.panelBody)}</Text>
          )}
          <View className="mt-2">
            <AskButton state={state} asking={asking} onAsk={onAsk} />
          </View>
        </>
      )}
      {failed && <AskFailed />}
      <CancelOutcome justCancelled={justCancelled && !pending} cancelFailed={cancelFailed} />
    </View>
  );
}

/** The panel once the access is on: it says so, and opens the coach's Home. */
function SeatGranted({ onOpenHome }: { onOpenHome?: () => void }) {
  const { t } = useTranslation();
  return (
    <View
      className="mx-4 rounded-xl border border-border-faint bg-surface-container-lowest p-4 gap-1.5"
      testID="coach-seat-panel"
    >
      <View className="flex-row items-center gap-3">
        <SeatGlyph kind="granted" />
        <Text
          className="flex-1 text-base font-semibold text-text-primary"
          accessibilityRole="header"
          testID="coach-seat-panel-granted"
        >
          {t(COACH_SEAT_KEYS.grantedTitle)}
        </Text>
      </View>
      <Text className="text-sm text-text-secondary">{t(COACH_SEAT_KEYS.grantedBody)}</Text>
      {onOpenHome ? (
        <View className="mt-2">
          <Button title={t(COACH_SEAT_KEYS.openHome)} onPress={onOpenHome} fullWidth testID="coach-seat-open-home" />
        </View>
      ) : null}
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
 * the Dravr team reviewing — a person, never automatically, the answer coming
 * here and by email, with no wait time promised; then the seat and what it
 * opens.
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
        detail={t(COACH_SEAT_KEYS.stepReviewNotice)}
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
