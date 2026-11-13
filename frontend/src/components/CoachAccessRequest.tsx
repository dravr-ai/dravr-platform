// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access request for a coachless group the coach owns — inline in onboarding, a strip over the room, a block in Group info
// ABOUTME: Shows the latest request's state (to ask, pending, declined) from the server; asking grants nothing (ADR-018)

import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Clock, UserCheck } from 'lucide-react';
import { clsx } from 'clsx';
import {
  COACH_PLATFORM_PROVIDERS,
  COACH_SEAT_CHECK_POLL_MS,
  COACH_SEAT_CHECK_WINDOW_MS,
  COACH_SEAT_KEYS,
  coachSeatMoment,
  coachSeatState,
} from '@pierre/shared-constants';
import type { CoachAccessRequest as CoachAccessRequestRecord } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import { providersApi, userApi } from '../services/api';
import { QUERY_KEYS } from '../constants/queryKeys';
import { Button } from './ui';
import ProviderConnectionCards from './ProviderConnectionCards';

/**
 * Where the request is shown.
 *
 * - `inline`: the onboarding group step — the button, or the state, under the
 *   step's own copy.
 * - `strip`: the top of the group room, where the coach is blocked: what they
 *   are not yet, the one filled request, and the way to Group info.
 * - `block`: the first block of Group info: what is blocked, the drawer's one
 *   filled request, then the coaching platforms as plain rows; once asked, the
 *   dated three-step timeline.
 */
export type CoachAccessRequestVariant = 'inline' | 'strip' | 'block';

interface CoachAccessRequestProps {
  groupId: string;
  variant?: CoachAccessRequestVariant;
  /** `strip` only: open Group info, where the coaching platforms are offered. */
  onOpenInfo?: () => void;
}

/**
 * The one coach-access request for a coachless group (carnet#738), behind
 * every entry point so none can drift: the onboarding group step, the room's
 * top strip and Group info.
 *
 * Reads where the coach's latest request stands so a return visit shows it.
 * The request names this group, so a grant attaches the coach to it. Asking
 * grants nothing (ADR-018): a super-admin decides, and the pending state says
 * so — it never promises a notification the product does not send.
 */
export default function CoachAccessRequest({ groupId, variant = 'inline', onOpenInfo }: CoachAccessRequestProps) {
  const queryClient = useQueryClient();
  // Set when a coaching platform was just linked here: an Intervals.icu roster
  // that coaches an active athlete files the request in the background
  // (`coach_access_evidence`), a few seconds after the link answers, so the
  // request is read again for the seat step's window. The block shows what the
  // server filed, never a guess.
  const [probeUntil, setProbeUntil] = useState<number | null>(null);
  const startProbe = useCallback(() => setProbeUntil(Date.now() + COACH_SEAT_CHECK_WINDOW_MS), []);
  const latest = useQuery({
    queryKey: QUERY_KEYS.user.coachAccessRequest(),
    queryFn: () => userApi.getCoachAccessRequest(),
    refetchInterval: (query) =>
      probeUntil !== null && Date.now() < probeUntil && coachSeatState(query.state.data?.request) === 'to_request'
        ? COACH_SEAT_CHECK_POLL_MS
        : false,
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
  // showing a request already waiting. A failed read offers the request:
  // the server keeps one pending request per account, so asking is harmless.
  if (latest.isLoading) return null;

  const view: SeatViewProps = {
    state: coachSeatState(request),
    request,
    asking: ask.isPending,
    failed: ask.isError,
    justAsked: ask.isSuccess,
    onAsk: () => ask.mutate(),
  };

  return variant === 'strip' ? (
    <SeatStrip {...view} onOpenInfo={onOpenInfo} />
  ) : (
    <SeatBlock {...view} onPlatformLinked={startProbe} />
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
      <p className="mt-3 text-sm text-on-surface" role="status" data-testid="coach-access-pending">
        {t('humanCoach.requestAccessPending')}
      </p>
    );
  }

  return (
    <div className="mt-3 space-y-2">
      {status === 'declined' ? (
        <p className="text-sm text-on-surface-variant" data-testid="coach-access-declined">
          {t('humanCoach.requestAccessDeclined')}
        </p>
      ) : null}
      <Button
        variant="secondary"
        onClick={() => ask.mutate()}
        disabled={ask.isPending || loading}
        data-testid="coach-access-request"
      >
        {ask.isPending ? t('humanCoach.requestAccessSending') : t('humanCoach.requestAccess')}
      </Button>
      {ask.isError ? (
        <p className="text-sm text-error" role="alert">
          {t('humanCoach.requestAccessFailed')}
        </p>
      ) : null}
    </div>
  );
}

interface SeatViewProps {
  state: ReturnType<typeof coachSeatState>;
  request: CoachAccessRequestRecord | null;
  asking: boolean;
  failed: boolean;
  /** The request was just sent from this view: focus moves to its new state. */
  justAsked: boolean;
  onAsk: () => void;
}

/** The filled request — "Request coach access", or "Ask again" after a decline. */
function AskButton({ state, asking, onAsk, fullWidthOnPhone }: Pick<SeatViewProps, 'state' | 'asking' | 'onAsk'> & { fullWidthOnPhone?: boolean }) {
  const { t } = useTranslation();
  return (
    <Button
      variant="primary"
      onClick={onAsk}
      disabled={asking}
      className={clsx(fullWidthOnPhone && 'w-full sm:w-auto')}
      data-testid="coach-access-request"
    >
      {asking ? t(COACH_SEAT_KEYS.sending) : state === 'declined' ? t(COACH_SEAT_KEYS.askAgain) : t(COACH_SEAT_KEYS.request)}
    </Button>
  );
}

/** The request failed: what happened and that trying again is the way out. */
function AskFailed() {
  const { t } = useTranslation();
  return (
    <p className="text-sm text-error" role="alert" data-testid="coach-access-failed">
      {t(COACH_SEAT_KEYS.failed)}
    </p>
  );
}

/** A round glyph beside a strip's title: the seat to take, or the clock while the team reviews. */
function SeatGlyph({ pending }: { pending: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={clsx(
        'flex h-9 w-9 shrink-0 items-center justify-center rounded-full',
        pending ? 'bg-warning/15 text-on-warning-container' : 'bg-primary-container text-on-primary-container',
      )}
    >
      {pending ? <Clock className="h-[18px] w-[18px]" /> : <UserCheck className="h-[18px] w-[18px]" />}
    </span>
  );
}

/** Focus a heading once the request this view sent comes back, so the keyboard lands on the new state. */
function useFocusOnAsked(justAsked: boolean, state: SeatViewProps['state']) {
  const ref = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (justAsked && state === 'pending') ref.current?.focus();
  }, [justAsked, state]);
  return ref;
}

/**
 * The room's top strip: the coach sees, without opening anything, that they
 * are not this group's coach yet and the one filled way to ask. Once asked it
 * dates the request and says who reviews it and how long that usually takes.
 */
function SeatStrip({ state, request, asking, failed, justAsked, onAsk, onOpenInfo }: SeatViewProps & { onOpenInfo?: () => void }) {
  const { t, language } = useTranslation();
  const titleId = useId();
  const headingRef = useFocusOnAsked(justAsked, state);
  const pending = state === 'pending';
  const sent = request ? coachSeatMoment(request.created_at, language) : null;
  const decided = request?.decided_at ? coachSeatMoment(request.decided_at, language) : sent;

  return (
    <section
      aria-labelledby={titleId}
      data-testid="coach-seat-strip"
      data-state={state}
      className="mx-auto flex max-w-[720px] items-start gap-3.5 rounded-xl border ghost-border bg-surface-container-lowest p-4"
    >
      <SeatGlyph pending={pending} />
      <div className="min-w-0 flex-1 space-y-1">
        <h2
          id={titleId}
          ref={headingRef}
          tabIndex={-1}
          className="font-display text-base font-semibold text-on-surface focus:outline-none"
        >
          {pending && sent
            ? t(COACH_SEAT_KEYS.pendingTitle, { date: sent.date })
            : state === 'declined'
              ? t(COACH_SEAT_KEYS.declinedTitle)
              : t(COACH_SEAT_KEYS.title)}
        </h2>
        {pending ? (
          <p className="text-sm text-on-surface-variant" data-testid="coach-access-pending">
            {t(COACH_SEAT_KEYS.pendingBody)}
          </p>
        ) : state === 'declined' && decided ? (
          <p className="text-sm text-on-surface-variant" data-testid="coach-access-declined">
            {t(COACH_SEAT_KEYS.declinedBody, { date: decided.date })}
          </p>
        ) : (
          <p className="text-sm text-on-surface-variant">{t(COACH_SEAT_KEYS.stripBody)}</p>
        )}
        {!pending && (
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2 pt-2">
            <AskButton state={state} asking={asking} onAsk={onAsk} fullWidthOnPhone />
            {onOpenInfo && (
              <button
                type="button"
                onClick={onOpenInfo}
                className="btn-base btn-tertiary w-full px-1.5 sm:w-auto"
                data-testid="coach-seat-open-info"
              >
                {t(COACH_SEAT_KEYS.linkAccount)}
              </button>
            )}
          </div>
        )}
        {failed && <AskFailed />}
      </div>
    </section>
  );
}

/**
 * Group info's first block: the state as its title, one line on what is
 * blocked, the drawer's one filled request, then TrainingPeaks and
 * Intervals.icu as plain identical rows. After a request, the dated three-step
 * timeline in its place; after a decline, the date and "Ask again".
 */
function SeatBlock({ state, request, asking, failed, justAsked, onAsk, onPlatformLinked }: SeatViewProps & { onPlatformLinked: () => void }) {
  const { t, language } = useTranslation();
  const titleId = useId();
  const headingRef = useFocusOnAsked(justAsked, state);
  usePlatformLinked(state === 'pending' ? null : onPlatformLinked);
  const pending = state === 'pending';
  const decided = request ? coachSeatMoment(request.decided_at ?? request.created_at, language) : null;

  return (
    <section
      aria-labelledby={titleId}
      data-testid="coach-seat-block"
      data-state={state}
      className="space-y-2 rounded-xl border ghost-border bg-surface-container-lowest p-4"
    >
      <h3
        id={titleId}
        ref={headingRef}
        tabIndex={-1}
        className="font-display text-lg font-semibold text-on-surface focus:outline-none"
      >
        {pending
          ? t(COACH_SEAT_KEYS.pendingBlockTitle)
          : state === 'declined'
            ? t(COACH_SEAT_KEYS.declinedTitle)
            : t(COACH_SEAT_KEYS.title)}
      </h3>

      {pending && request ? (
        <SeatTimeline request={request} />
      ) : (
        <>
          {state === 'declined' && decided ? (
            <p className="text-sm text-on-surface-variant" data-testid="coach-access-declined">
              {t(COACH_SEAT_KEYS.declinedBody, { date: decided.date })}
            </p>
          ) : (
            <p className="text-sm text-on-surface-variant">{t(COACH_SEAT_KEYS.blockBody)}</p>
          )}
          <div className="pt-1.5">
            <AskButton state={state} asking={asking} onAsk={onAsk} />
          </div>
          {failed && <AskFailed />}
          <div className="!mt-4 border-t ghost-border pt-3" data-testid="coach-seat-routes">
            <p className="text-xs text-outline">{t(COACH_SEAT_KEYS.routesLabel)}</p>
            <ProviderConnectionCards onlyProviders={COACH_PLATFORM_PROVIDERS} plain />
          </div>
        </>
      )}
    </section>
  );
}

/**
 * Call `onLinked` when a coaching platform becomes connected while the block
 * is on screen — not for one connected before it opened. Reads the provider
 * status the rows themselves read, so no extra request is made.
 */
function usePlatformLinked(onLinked: (() => void) | null) {
  const { data } = useQuery({
    queryKey: QUERY_KEYS.providers.status(),
    queryFn: () => providersApi.getProvidersStatus(),
  });
  const linked = (data?.providers ?? [])
    .filter((p) => p.connected && COACH_PLATFORM_PROVIDERS.includes(p.provider))
    .map((p) => p.provider)
    .sort()
    .join(',');
  const seen = useRef<string | null>(null);
  useEffect(() => {
    if (!data) return;
    const before = seen.current;
    seen.current = linked;
    if (before === null || !onLinked) return;
    const grew = linked.split(',').some((p) => p && !before.split(',').includes(p));
    if (grew) onLinked();
  }, [data, linked, onLinked]);
}

/**
 * The pending request as three steps (P1·C): sent, with its date and time;
 * the Dravr team reviewing, usually within 24 hours; then the seat and what
 * it opens. It promises no notification: a grant reaches the operators today.
 */
function SeatTimeline({ request }: { request: CoachAccessRequestRecord }) {
  const { t, language } = useTranslation();
  const sent = coachSeatMoment(request.created_at, language);
  return (
    <ol
      aria-label={t(COACH_SEAT_KEYS.timelineLabel)}
      className="flex flex-col pt-1"
      data-testid="coach-access-pending"
    >
      <TimelineStep
        marker="done"
        title={t(COACH_SEAT_KEYS.stepSent)}
        status={t(COACH_SEAT_KEYS.stepDone)}
        detail={t(COACH_SEAT_KEYS.stepSentWhen, { date: sent.date, time: sent.time })}
        testId="coach-seat-step-sent"
      />
      <TimelineStep
        marker="current"
        title={t(COACH_SEAT_KEYS.stepReview)}
        status={t(COACH_SEAT_KEYS.stepNow)}
        detail={t(COACH_SEAT_KEYS.stepReviewWhen)}
        testId="coach-seat-step-review"
      />
      <TimelineStep
        marker="upcoming"
        title={t(COACH_SEAT_KEYS.stepSeat)}
        detail={t(COACH_SEAT_KEYS.stepSeatWhat)}
        last
        testId="coach-seat-step-seat"
      />
    </ol>
  );
}

function TimelineStep({
  marker,
  title,
  status,
  detail,
  last = false,
  testId,
}: {
  marker: 'done' | 'current' | 'upcoming';
  title: string;
  status?: string;
  detail: string;
  last?: boolean;
  testId: string;
}) {
  return (
    <li className="flex gap-3" aria-current={marker === 'current' ? 'step' : undefined} data-testid={testId}>
      <div aria-hidden="true" className="flex shrink-0 flex-col items-center">
        {marker === 'done' ? (
          <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary text-on-primary">
            <Check className="h-3.5 w-3.5" strokeWidth={2.4} />
          </span>
        ) : marker === 'current' ? (
          <span className="flex h-6 w-6 items-center justify-center rounded-full border-2 border-primary bg-surface-container-lowest">
            <span className="h-2 w-2 rounded-full bg-primary" />
          </span>
        ) : (
          <span className="h-6 w-6 rounded-full border-2 border-outline bg-surface-container-lowest" />
        )}
        {!last && (
          <span className={clsx('min-h-4 w-0.5 flex-1', marker === 'done' ? 'bg-primary' : 'bg-outline-variant')} />
        )}
      </div>
      <div className={clsx('min-w-0 pt-0.5', last ? 'pb-1' : 'pb-4')}>
        <p className="text-sm font-semibold text-on-surface">
          {title}
          {status && <span className="font-normal text-on-surface-variant">{` · ${status}`}</span>}
        </p>
        <p className="text-sm text-on-surface-variant">{detail}</p>
      </div>
    </li>
  );
}
