// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's review of what Dravr sent their athletes — every group reply, its verdicts, a flag and a correction
// ABOUTME: Replies go out unreviewed, so the coach steps in after the fact: a flag stays theirs, a correction joins the thread (carnet#847)

import { useMemo, useState } from 'react';
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Flag, PenLine } from 'lucide-react';
import type { CoachReply, ReplyFlagReason, VerdictTone } from '@pierre/shared-types';
import { formatDateTime } from '@pierre/chat-utils';
import {
  COACH_REPLIES_PAGE_SIZE,
  MAX_CORRECTION_CHARS,
  MAX_FLAG_NOTE_CHARS,
  REPLY_FLAG_REASON_LABEL_KEY as REASON_LABEL_KEY,
  REPLY_FLAG_REASONS as REASONS,
  VERDICT_STATUS_LABEL_KEY,
  coachReviewWeekSummary,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { ALL_COACHED_ATHLETES, coachedAthleteOptions, describeApiError, replyStatus } from '@pierre/ui-logic';
import { coachRepliesApi } from '../../services/api';
import { QUERY_KEYS } from '../../constants/queryKeys';
import { TabHeader } from '../ui/TabHeader';
import { Button, Select, Textarea, useErrorToast, useSuccessToast } from '../ui';
import { StatusDot, type DotTone } from '../coach-home/parts';

/** The dot each verdict rollup tone draws; a rhetorical or unverifiable claim is no alarm, so it stays quiet. */
const VERDICT_DOT_TONE: Record<VerdictTone, DotTone> = {
  success: 'ok',
  warning: 'warn',
  error: 'bad',
  info: 'none',
  secondary: 'none',
};

/** Which form a reply card has open. */
type OpenForm = 'flag' | 'correct' | null;

interface ReplyCardProps {
  reply: CoachReply;
}

/** One reply: who it went to, what it said, what the checker found, and the coach's own steps. */
function ReplyCard({ reply }: ReplyCardProps) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const showSuccess = useSuccessToast();
  const showError = useErrorToast();
  const [open, setOpen] = useState<OpenForm>(null);
  const [reason, setReason] = useState<ReplyFlagReason>('wrong');
  const [note, setNote] = useState('');
  const [correction, setCorrection] = useState('');
  const athlete = reply.athlete_name ?? t('coachReview.unnamedAthlete');
  const status = replyStatus(t, reply, athlete, i18n.language);

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.coachReplies.all });
  };

  const flag = useMutation({
    mutationFn: () => coachRepliesApi.flag(reply.id, reason, note),
    onSuccess: () => {
      setOpen(null);
      setNote('');
      showSuccess(t('coachReview.flagSent'));
      refresh();
    },
    onError: (error) => showError(describeApiError(error, { t, fallbackKey: 'coachReview.flagFailed' })),
  });

  const correct = useMutation({
    mutationFn: () => coachRepliesApi.correct(reply.id, correction.trim()),
    onSuccess: () => {
      setOpen(null);
      setCorrection('');
      showSuccess(t('coachReview.correctionSent'));
      refresh();
      // The athlete's thread shows the correction; a thread this browser has
      // open reads the room again.
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.room(reply.group_id) });
    },
    onError: (error) =>
      showError(describeApiError(error, { t, fallbackKey: 'coachReview.correctionFailed' })),
  });

  return (
    <article data-testid={`coach-reply-${reply.id}`} className="card px-4 py-3 sm:px-5">
      <header className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <p className="min-w-0 truncate text-sm font-medium text-on-surface">
          {t('coachReview.replyTo', { name: athlete, group: reply.group_name })}
        </p>
        <time dateTime={reply.created_at} className="text-xs text-outline">
          {formatDateTime(reply.created_at, i18n.language)}
        </time>
      </header>

      {reply.withheld || reply.content === null ? (
        <p className="mt-2 text-sm italic text-on-surface-variant">{t('coachReview.withheld')}</p>
      ) : (
        <p data-testid="coach-reply-content" className="mt-2 whitespace-pre-wrap break-words text-sm leading-relaxed text-on-surface">
          {reply.content}
        </p>
      )}

      {/* One status per reply, one dot beside its words, never red or green
          text (DESIGN.md, Coach Home); the claims follow, each as its words. */}
      {status !== null && (
        <div data-testid="coach-reply-verdicts" className="mt-2 space-y-1">
          <p
            data-testid="coach-reply-status"
            aria-label={t('app.claimVerdictsStatus', { status: status.spoken })}
            className="flex items-start gap-2 text-xs font-medium text-on-surface"
          >
            <span className="flex h-4 items-center">
              <StatusDot tone={VERDICT_DOT_TONE[status.tone]} />
            </span>
            <span className="min-w-0">{status.text}</span>
          </p>
          {reply.verdicts.length > 0 && (
            <ul aria-label={t('coachReview.verdicts')} className="space-y-0.5">
              {reply.verdicts.map((verdict) => (
                <li key={verdict.id} data-testid="coach-reply-verdict" className="break-words text-xs text-on-surface-variant">
                  {t(VERDICT_STATUS_LABEL_KEY[verdict.status])} · {verdict.claim_text}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {reply.flags.map((f) => (
        <p key={f.id} data-testid="coach-reply-flag" className="mt-2 text-xs text-on-surface-variant">
          <Flag className="mr-1 inline h-3 w-3" aria-hidden="true" />
          {t('coachReview.flagged', { reason: t(REASON_LABEL_KEY[f.reason]) })}
          {f.note ? ` — ${f.note}` : ''}
        </p>
      ))}
      {reply.corrections.map((c) => (
        <div key={c.id} data-testid="coach-reply-correction" className="mt-2 border-l-2 border-primary pl-3">
          <p className="text-xs font-medium text-primary">{t('coachReview.correctionPosted')}</p>
          <p className="whitespace-pre-wrap break-words text-sm text-on-surface">{c.content}</p>
        </div>
      ))}

      {!reply.withheld && open === null && (
        <div className="mt-3 flex gap-4">
          <button type="button" className="btn-tertiary btn-sm" onClick={() => setOpen('flag')}>
            <Flag className="h-3.5 w-3.5" aria-hidden="true" />
            {t('coachReview.flag')}
          </button>
          <button type="button" className="btn-tertiary btn-sm" onClick={() => setOpen('correct')}>
            <PenLine className="h-3.5 w-3.5" aria-hidden="true" />
            {t('coachReview.correct')}
          </button>
        </div>
      )}

      {open === 'flag' && (
        <form
          className="mt-3 space-y-3"
          onSubmit={(event) => {
            event.preventDefault();
            flag.mutate();
          }}
        >
          <Select
            label={t('coachReview.reasonLabel')}
            value={reason}
            onChange={(event) => setReason(event.target.value as ReplyFlagReason)}
            options={REASONS.map((value) => ({ value, label: t(REASON_LABEL_KEY[value]) }))}
          />
          <Textarea
            label={t('coachReview.noteLabel')}
            value={note}
            onChange={(event) => setNote(event.target.value)}
            maxLength={MAX_FLAG_NOTE_CHARS}
            rows={2}
          />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="secondary" size="sm" onClick={() => setOpen(null)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" size="sm" loading={flag.isPending}>
              {t('coachReview.submitFlag')}
            </Button>
          </div>
        </form>
      )}

      {open === 'correct' && (
        <form
          className="mt-3 space-y-3"
          onSubmit={(event) => {
            event.preventDefault();
            if (correction.trim()) correct.mutate();
          }}
        >
          <Textarea
            label={t('coachReview.correctionLabel')}
            placeholder={t('coachReview.correctionPlaceholder', { name: athlete })}
            value={correction}
            onChange={(event) => setCorrection(event.target.value)}
            maxLength={MAX_CORRECTION_CHARS}
            rows={3}
          />
          <div className="flex justify-end gap-2">
            <Button type="button" variant="secondary" size="sm" onClick={() => setOpen(null)}>
              {t('common.cancel')}
            </Button>
            <Button type="submit" size="sm" loading={correct.isPending} disabled={!correction.trim()}>
              {t('coachReview.submitCorrection')}
            </Button>
          </div>
        </form>
      )}
    </article>
  );
}

/**
 * Every reply Dravr sent into a group the coach runs, newest first, narrowed
 * to one athlete. The header line counts this week's flags and corrections —
 * the "corrections needed" figure the cohort report prints.
 */
export default function CoachReplies({ initialAthleteId = null }: { initialAthleteId?: string | null }) {
  const { t, i18n } = useTranslation();
  const [athleteId, setAthleteId] = useState<string | null>(initialAthleteId);

  const pages = useInfiniteQuery({
    queryKey: QUERY_KEYS.coachReplies.list(athleteId),
    queryFn: ({ pageParam }) =>
      coachRepliesApi.list({
        athlete_id: athleteId ?? undefined,
        before: pageParam ?? undefined,
        limit: COACH_REPLIES_PAGE_SIZE,
      }),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next_before,
  });
  const weekly = useQuery({
    queryKey: QUERY_KEYS.coachReplies.weekly(),
    queryFn: () => coachRepliesApi.weekly(1),
  });

  const first = pages.data?.pages[0];
  const replies = useMemo(() => pages.data?.pages.flatMap((page) => page.replies) ?? [], [pages.data]);
  const thisWeek = weekly.data?.weeks.at(-1);
  const description = thisWeek ? coachReviewWeekSummary(t, thisWeek, i18n.language) : undefined;

  const filter = first && first.athletes.length > 0 && (
    <Select
      aria-label={t('coachReview.athleteFilter')}
      size="sm"
      value={athleteId ?? ''}
      onChange={(event) => setAthleteId(event.target.value || null)}
      // The reader's collation, so « Félix » sorts with the F's (the server sends code-point order).
      options={coachedAthleteOptions(t, i18n.language, first.athletes).map((option) => ({
        value: option.key === ALL_COACHED_ATHLETES ? '' : option.key,
        label: option.label,
      }))}
    />
  );

  let body;
  if (pages.isLoading) {
    body = (
      <div className="flex items-center justify-center py-16">
        <div className="pierre-spinner" />
      </div>
    );
  } else if (pages.isError) {
    body = <p className="px-4 py-3 text-sm text-error sm:px-6">{t('coachReview.loadFailed')}</p>;
  } else if (first && first.groups.length === 0) {
    body = (
      <p data-testid="coach-replies-not-a-coach" className="px-4 py-3 text-sm text-on-surface-variant sm:px-6">
        {t('coachReview.notACoach')}
      </p>
    );
  } else if (replies.length === 0) {
    body = (
      <p data-testid="coach-replies-empty" className="px-4 py-3 text-sm text-on-surface-variant sm:px-6">
        {t('coachReview.empty')}
      </p>
    );
  } else {
    body = (
      <div className="mx-auto flex max-w-[720px] flex-col gap-3 px-4 py-4 sm:px-6">
        {replies.map((reply) => (
          <ReplyCard key={reply.id} reply={reply} />
        ))}
        {pages.hasNextPage && (
          <button
            type="button"
            className="btn-tertiary self-center"
            disabled={pages.isFetchingNextPage}
            onClick={() => void pages.fetchNextPage()}
          >
            {t('coachReview.loadOlder')}
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <TabHeader title={t('coachReview.title')} description={description} actions={filter || undefined} />
      <div className="flex-1 overflow-y-auto">{body}</div>
    </div>
  );
}
