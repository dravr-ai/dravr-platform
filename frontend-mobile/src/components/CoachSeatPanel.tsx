// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-seat step's lower half (mobile): the link to ask the team, what the server did, and the way on
// ABOUTME: Mirrors the web OnboardingCoachSeat; reads the seat back (session, request, account role), never infers it from a tap

import React, { useEffect, useState } from 'react';
import { View, Text, Pressable, ActivityIndicator } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import {
  COACH_SEAT_CHECK_POLL_MS,
  COACH_SEAT_CHECK_WINDOW_MS,
  QUERY_KEYS,
  coachSeatMoment,
  coachSeatState,
  coachSeatStepState,
  connectedCoachPlatforms,
  type CoachSeatProvider,
  type CoachSeatStepState,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { Button } from './ui';
import { authApi, userApi } from '../services/api';
import { useAuth } from '../contexts/AuthContext';
import { useThemeColors } from '../constants/theme';

/**
 * Below the two coaching-platform rows the connect screen draws in its seat
 * mode: « Je n'utilise ni l'un ni l'autre → » (a request with no group; the
 * group made next is named on it by the server), the seat as the server has
 * it once there is something to say, and Continue — or « Passer pour
 * l'instant » while there is nothing.
 *
 * Both connections settle the seat off their connect request, so while one is
 * connected the panel reads the server again for a short while — the session
 * for `manages_roster`, the request, the cards for the account role
 * (`onReloadProviders`) — before saying a connection produced nothing.
 */
export function CoachSeatPanel({
  providers,
  onReloadProviders,
  onDone,
}: {
  providers: readonly CoachSeatProvider[];
  onReloadProviders: () => void;
  onDone: () => void;
}) {
  const { t, language } = useTranslation();
  const queryClient = useQueryClient();
  const { user, updateUser } = useAuth();
  const managesRoster = user?.manages_roster === true;
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const ask = useMutation({
    mutationFn: () => userApi.requestCoachAccess(),
    onSuccess: (data) => queryClient.setQueryData(QUERY_KEYS.user.coachAccessRequest(), data),
  });

  const platformConnected = connectedCoachPlatforms(providers).length > 0;
  const [checkExpired, setCheckExpired] = useState(false);
  const seat = coachSeatStepState({ managesRoster, request: latest.data?.request, providers, checkExpired });

  useEffect(() => {
    if (!platformConnected) return;
    const timer = setTimeout(() => setCheckExpired(true), COACH_SEAT_CHECK_WINDOW_MS);
    return () => clearTimeout(timer);
  }, [platformConnected]);

  const awaiting =
    platformConnected && !managesRoster && coachSeatState(latest.data?.request) !== 'pending' && !checkExpired;
  const refetchRequest = latest.refetch;
  useEffect(() => {
    if (!awaiting) return;
    let cancelled = false;
    const read = () => {
      onReloadProviders();
      void refetchRequest();
      authApi
        .getSession()
        .then(({ user: fresh }) => {
          if (!cancelled && fresh.manages_roster === true) void updateUser({ manages_roster: true });
        })
        // The next tick reads again.
        .catch(() => {});
    };
    read();
    const timer = setInterval(read, COACH_SEAT_CHECK_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [awaiting, onReloadProviders, refetchRequest, updateUser]);

  return (
    <View>
      {seat.kind === 'open' ? (
        <Pressable
          onPress={() => ask.mutate()}
          disabled={ask.isPending || latest.isLoading}
          accessibilityRole="button"
          className="min-h-11 justify-center px-4"
          testID="coach-seat-ask"
        >
          <Text className="text-base font-medium text-primary">
            {ask.isPending ? t('humanCoach.requestAccessSending') : t('humanCoach.seatNeither')}
          </Text>
        </Pressable>
      ) : (
        <SeatStatus
          seat={seat}
          sentOn={(iso) => coachSeatMoment(iso, language).date}
          asking={ask.isPending}
          onAsk={() => ask.mutate()}
        />
      )}

      {ask.isError ? (
        <Text className="mt-3 px-4 text-sm text-error" accessibilityRole="alert" testID="coach-seat-ask-failed">
          {t('humanCoach.requestAccessFailed')}
        </Text>
      ) : null}

      <View className="mt-8 px-4">
        {seat.kind === 'open' ? (
          <Button title={t('humanCoach.seatSkip')} variant="ghost" onPress={onDone} testID="coach-seat-skip" />
        ) : (
          <Button title={t('onboarding.continueButton')} onPress={onDone} fullWidth testID="coach-seat-continue" />
        )}
      </View>
    </View>
  );
}

/** The seat as the server has it, once there is something to say. */
function SeatStatus({
  seat,
  sentOn,
  asking,
  onAsk,
}: {
  seat: Exclude<CoachSeatStepState, { kind: 'open' }>;
  sentOn: (iso: string) => string;
  asking: boolean;
  onAsk: () => void;
}) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const askAgain = (
    <Pressable
      onPress={onAsk}
      disabled={asking}
      accessibilityRole="button"
      className="mt-1 min-h-11 justify-center"
      testID="coach-seat-ask"
    >
      <Text className="text-base font-medium text-primary">
        {asking ? t('humanCoach.requestAccessSending') : t('humanCoach.seatAskTeam')}
      </Text>
    </Pressable>
  );

  return (
    <View
      className="mx-4 mt-6 rounded-xl border border-border-faint bg-surface-container-lowest px-4 py-3"
      accessibilityLiveRegion="polite"
      testID={`coach-seat-status-${seat.kind}`}
    >
      {seat.kind === 'coach' ? (
        <View className="flex-row items-center gap-2">
          <Feather name="check-circle" size={20} color={colors.success} />
          <Text className="flex-1 text-base font-semibold text-on-surface">{t('humanCoach.seatCoach')}</Text>
        </View>
      ) : seat.kind === 'requested' ? (
        <>
          <Text className="text-base font-semibold text-on-surface">{t('humanCoach.seatRequestDone')}</Text>
          <Text className="mt-1 text-sm text-on-surface-variant">
            {t('humanCoach.seatRequestSentNotice', { date: sentOn(seat.sentAt) })}
          </Text>
        </>
      ) : seat.kind === 'checking' ? (
        <View className="flex-row items-center gap-2">
          <ActivityIndicator size="small" color={colors.tokens.primary} />
          <Text className="flex-1 text-sm text-on-surface-variant">{t('humanCoach.seatChecking')}</Text>
        </View>
      ) : seat.kind === 'declined' ? (
        <>
          <Text className="text-sm text-on-surface">{t('humanCoach.requestAccessDeclined')}</Text>
          {askAgain}
        </>
      ) : (
        <>
          <Text className="text-sm text-on-surface">{t('humanCoach.seatNothing', { provider: seat.provider })}</Text>
          {askAgain}
        </>
      )}
    </View>
  );
}
