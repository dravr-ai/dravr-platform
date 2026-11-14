// ABOUTME: Onboarding step for a coach — get the coach seat first: connect TrainingPeaks or Intervals.icu, or ask the team
// ABOUTME: Says only what the server did (coach access, a dated request, nothing); asking grants nothing (ADR-018)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useTranslation } from '@pierre/i18n';
import {
  COACH_PLATFORM_PROVIDERS,
  COACH_SEAT_CHECK_POLL_MS,
  COACH_SEAT_CHECK_WINDOW_MS,
  coachSeatMoment,
  coachSeatState,
  coachSeatStepState,
  connectedCoachPlatforms,
  type CoachSeatStepState,
} from '@pierre/shared-constants';
import { authApi, providersApi, userApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { useAuth } from '../hooks/useAuth';
import { Button } from './ui';
import OnboardingShell from './OnboardingShell';
import ProviderConnectionCards from './ProviderConnectionCards';

/**
 * The coach's seat, before anything is made (canvas P3·A, decision 3): the
 * two coaching platforms as plain provider rows, one link to ask the Dravr
 * team, and a way past.
 *
 * TrainingPeaks and Intervals.icu look alike here on purpose: the difference
 * is what the server does once one is connected — a TrainingPeaks coach
 * account on the user's verified email is coach access with no human in the
 * loop, an Intervals.icu roster that coaches an active athlete files the
 * request — so the step reads the outcome back (`manages_roster` from the
 * session, the latest coach-access request, each card's account role) and
 * never infers it from the row tapped. Both are written off the connect
 * request, so it reads again for a short while before saying a connection
 * produced nothing.
 *
 * « Je n'utilise ni l'un ni l'autre » files the request with no group; the
 * group made next is named on it by the server, so a grant attaches the coach
 * to it. A coach who already holds the seat when the step comes up is stepped
 * past without a screen.
 */
export default function OnboardingCoachSeat({ onComplete }: { onComplete: () => void }) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const { user, updateUser } = useAuth();
  const managesRoster = user?.manages_roster === true;

  // Held before the step came up: nothing to get. Read once, so a seat
  // granted on this screen is shown rather than skipped.
  const heldOnArrival = useRef(managesRoster);
  useEffect(() => {
    if (heldOnArrival.current) onComplete();
  }, [onComplete]);

  const providers = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
  });
  const ask = useMutation({
    mutationFn: () => userApi.requestCoachAccess(),
    onSuccess: (data) => queryClient.setQueryData(QUERY_KEYS.user.coachAccessRequest(), data),
  });

  const served = providers.data?.providers ?? [];
  const platformConnected = connectedCoachPlatforms(served).length > 0;
  const [checkExpired, setCheckExpired] = useState(false);
  const seat = coachSeatStepState({
    managesRoster,
    request: latest.data?.request,
    providers: served,
    checkExpired,
  });

  // The window for a connection's outcome opens the first time this screen
  // sees a coaching platform connected (back from a sign-in, a key, or an
  // authorization that reloaded the page).
  useEffect(() => {
    if (!platformConnected) return;
    const timer = window.setTimeout(() => setCheckExpired(true), COACH_SEAT_CHECK_WINDOW_MS);
    return () => window.clearTimeout(timer);
  }, [platformConnected]);

  // While a connection may still settle the seat, read the server again: the
  // session for `manages_roster`, the request, and the cards for the account
  // role. Stops at coach access, a waiting request, or the window's end.
  const awaiting =
    platformConnected && !managesRoster && coachSeatState(latest.data?.request) !== 'pending' && !checkExpired;
  const refetchProviders = providers.refetch;
  const refetchRequest = latest.refetch;
  useEffect(() => {
    if (!awaiting) return;
    let cancelled = false;
    const read = () => {
      void refetchProviders();
      void refetchRequest();
      authApi
        .getSession()
        .then(({ user: fresh }) => {
          if (!cancelled && fresh.manages_roster === true) updateUser({ manages_roster: true });
        })
        // The next tick reads again.
        .catch(() => {});
    };
    read();
    const timer = window.setInterval(read, COACH_SEAT_CHECK_POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [awaiting, refetchProviders, refetchRequest, updateUser]);

  if (heldOnArrival.current) return null;

  const settled = seat.kind !== 'open';
  return (
    <OnboardingShell heading={t('humanCoach.seatHeading')}>
      <div className="mx-auto max-w-md">
        <p className="mt-3 text-sm text-on-surface-variant text-center">
          {t('humanCoach.seatIntro')}
        </p>

        <div className="mt-8" data-testid="coach-seat-providers">
          <ProviderConnectionCards onlyProviders={COACH_PLATFORM_PROVIDERS} plain />
        </div>

        {seat.kind === 'open' ? (
          <button
            type="button"
            onClick={() => ask.mutate()}
            disabled={ask.isPending || latest.isLoading}
            className="touch-target mt-2 flex min-h-8 items-center pl-9 text-sm font-medium text-primary transition-colors hover:text-primary-hover disabled:opacity-60"
            data-testid="coach-seat-ask"
          >
            {ask.isPending ? t('humanCoach.requestAccessSending') : t('humanCoach.seatNeither')}
          </button>
        ) : (
          <SeatStatus
            seat={seat}
            sentOn={(iso) => coachSeatMoment(iso, i18n.language).date}
            asking={ask.isPending}
            onAsk={() => ask.mutate()}
          />
        )}

        {ask.isError ? (
          <p className="mt-3 text-sm text-error" role="alert">
            {t('humanCoach.requestAccessFailed')}
          </p>
        ) : null}

        <div className="mt-8 flex flex-col items-center">
          {settled ? (
            <Button
              variant="primary"
              onClick={onComplete}
              className="w-full"
              data-testid="coach-seat-continue"
            >
              {t('onboarding.continueButton')}
            </Button>
          ) : (
            <Button variant="tertiary" onClick={onComplete} data-testid="coach-seat-skip">
              {t('humanCoach.seatSkip')}
            </Button>
          )}
        </div>
      </div>
    </OnboardingShell>
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
  const askAgain = (
    <button
      type="button"
      onClick={onAsk}
      disabled={asking}
      className="touch-target mt-2 flex min-h-8 items-center text-sm font-medium text-primary transition-colors hover:text-primary-hover disabled:opacity-60"
      data-testid="coach-seat-ask"
    >
      {asking ? t('humanCoach.requestAccessSending') : t('humanCoach.seatAskTeam')}
    </button>
  );

  return (
    <div
      className="mt-6 rounded-xl border ghost-border bg-surface-container-lowest px-4 py-3"
      role="status"
      data-testid="coach-seat-status"
      data-state={seat.kind}
    >
      {seat.kind === 'coach' ? (
        <p className="flex items-center gap-2 text-sm font-semibold text-on-surface">
          <CheckDisc />
          {t('humanCoach.seatCoach')}
        </p>
      ) : seat.kind === 'requested' ? (
        <>
          <p className="text-sm font-semibold text-on-surface">{t('humanCoach.seatRequestDone')}</p>
          <p className="mt-1 text-sm text-on-surface-variant">
            {t('humanCoach.seatRequestSentNotice', { date: sentOn(seat.sentAt) })}
          </p>
        </>
      ) : seat.kind === 'checking' ? (
        <p className="flex items-center gap-2 text-sm text-on-surface-variant">
          <span aria-hidden="true" className="pierre-spinner h-4 w-4 border-on-surface border-t-transparent" />
          {t('humanCoach.seatChecking')}
        </p>
      ) : seat.kind === 'declined' ? (
        <>
          <p className="text-sm text-on-surface">{t('humanCoach.requestAccessDeclined')}</p>
          {askAgain}
        </>
      ) : (
        <>
          <p className="text-sm text-on-surface">
            {t('humanCoach.seatNothing', { provider: seat.provider })}
          </p>
          {askAgain}
        </>
      )}
    </div>
  );
}

/** The success disc beside a seat the coach holds; the invite's seat card shows it too. */
export function CheckDisc() {
  return (
    <span
      aria-hidden="true"
      className="flex h-6 w-6 flex-shrink-0 items-center justify-center rounded-full bg-success/15 text-on-success-container"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
        <path d="m5 12.5 4.5 4.5L19 7.5" />
      </svg>
    </span>
  );
}
