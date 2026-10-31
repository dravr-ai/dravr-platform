// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Board A — À traiter: today's athletes, this week's, the calm ones as one line of names, and the athlete pane beside
// ABOUTME: Below 768 the list and the pane take turns, the pane carrying the back button; "Traité" can always be undone

import { useMemo } from 'react';
import { clsx } from 'clsx';
import { ChevronLeft } from 'lucide-react';
import type { CoachAttentionReason, CoachRosterAthlete } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  buildQueue,
  describeLine,
  firstQueued,
  queueCount,
  queueWhen,
  type CoachQueue,
  type QueueEntry,
} from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { useDismissAttention, useRestoreAttention } from '../../hooks/useCoachRoster';
import { EmptyState, useErrorToast } from '../ui';
import { AthleteAvatar } from './parts';
import { CoachAthletePane } from './CoachAthletePane';
import type { CoachDestinations } from './useCoachDestinations';

/** A character no sentence holds, standing where the buttons of the names go. */
const NAMES_SLOT = '\u0000';

interface CoachQueueViewProps {
  athletes: CoachRosterAthlete[];
  selectedId: string | null;
  onSelect: (athleteId: string | null) => void;
  onSeeAll: () => void;
  today: string;
  destinations: CoachDestinations;
}

function QueueRow({
  entry,
  selected,
  horizon,
  onPick,
}: {
  entry: QueueEntry;
  selected: boolean;
  horizon: 'today' | 'week';
  onPick: () => void;
}) {
  const { t, language } = useTranslation();
  const when = queueWhen(language, entry);
  return (
    <button
      type="button"
      aria-pressed={selected}
      data-testid={`coach-queue-row-${entry.athlete.user_id}`}
      onClick={onPick}
      className={clsx(
        'flex min-h-[60px] w-full items-center gap-3 rounded-[10px] px-2.5 py-2.5 text-left transition-colors focus-ring-visible',
        selected ? 'bg-surface-container-low' : 'hover:bg-surface-container-low/60',
      )}
    >
      <AthleteAvatar userId={entry.athlete.user_id} name={entry.athlete.name} muted={entry.athlete.status !== 'shared'} />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-sm font-semibold text-on-surface">{entry.athlete.name}</span>
        <span className="truncate text-sm text-on-surface-variant" data-testid="coach-queue-reason">
          {describeLine(t, language, entry.reason.key, entry.reason.params)}
        </span>
      </span>
      <span className="flex shrink-0 flex-col items-end gap-1.5">
        <span className="font-mono text-xs text-on-surface-variant">{when}</span>
        <span
          aria-hidden="true"
          className={clsx(
            'h-2 w-2 rounded-full',
            horizon === 'today' ? 'bg-warning' : 'box-border border-[1.5px] border-outline',
          )}
        />
      </span>
    </button>
  );
}

function QueueList({
  queue,
  selectedId,
  onSelect,
  onSeeAll,
  onUndo,
}: {
  queue: CoachQueue;
  selectedId: string | null;
  onSelect: (id: string) => void;
  onSeeAll: () => void;
  onUndo: (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => void;
}) {
  const { t, language } = useTranslation();
  const total = queue.today.length + queue.week.length + queue.treated.length + queue.calm.length + queue.notSharing.length;
  const calmNames = queue.calm;
  // The names are buttons, so the sentence is cut around where they go.
  const [notSharingLead, notSharingTail = ''] = t('coachHome.queue.notSharing', { names: NAMES_SLOT }).split(NAMES_SLOT);
  return (
    <div className="px-3 pb-6 pt-4" data-testid="coach-queue">
      <div className="flex items-baseline gap-2.5 px-2 pb-3">
        <h1 className="m-0 font-display text-xl font-semibold tracking-tight text-on-surface">{t('coachHome.queue.title')}</h1>
        <span className="font-mono text-xs text-on-surface-variant" data-testid="coach-queue-count">
          {queueCount(queue)}
        </span>
        <button
          type="button"
          data-testid="coach-see-all"
          onClick={onSeeAll}
          className="ml-auto inline-flex items-center gap-1.5 rounded text-sm font-medium text-primary hover:underline focus-ring-visible"
        >
          {t('coachHome.queue.seeAll')}
          <span className="font-mono text-xs font-normal">{total}</span>
        </button>
      </div>

      {queueCount(queue) === 0 && <EmptyState className="px-2" data-testid="coach-queue-empty">{t('coachHome.queue.empty')}</EmptyState>}

      {queue.today.length > 0 && (
        <section aria-label={t('coachHome.queue.today')} data-testid="coach-queue-today">
          <div className="px-2 pb-1.5 pt-2.5 text-xs font-medium text-on-surface-variant">{t('coachHome.queue.today')}</div>
          {queue.today.map((entry) => (
            <QueueRow
              key={entry.athlete.user_id}
              entry={entry}
              horizon="today"
              selected={entry.athlete.user_id === selectedId}
              onPick={() => onSelect(entry.athlete.user_id)}
            />
          ))}
        </section>
      )}

      {queue.week.length > 0 && (
        <section aria-label={t('coachHome.queue.week')} data-testid="coach-queue-week">
          <div className="px-2 pb-1.5 pt-4 text-xs font-medium text-on-surface-variant">{t('coachHome.queue.week')}</div>
          {queue.week.map((entry) => (
            <QueueRow
              key={entry.athlete.user_id}
              entry={entry}
              horizon="week"
              selected={entry.athlete.user_id === selectedId}
              onPick={() => onSelect(entry.athlete.user_id)}
            />
          ))}
        </section>
      )}

      {queue.treated.length > 0 && (
        <section aria-label={t('coachHome.queue.treated')} data-testid="coach-queue-treated" className="mt-4">
          <div className="flex items-baseline gap-2 px-2 pb-1.5 pt-2.5 text-xs font-medium text-on-surface-variant">
            {t('coachHome.queue.treated')}
            <span className="font-mono">{queue.treated.length}</span>
          </div>
          {queue.treated.map((entry) => (
            <div
              key={entry.athlete.user_id}
              className={clsx(
                'flex min-h-[48px] items-center gap-3 rounded-[10px] px-2.5 py-1.5',
                entry.athlete.user_id === selectedId && 'bg-surface-container-low',
              )}
            >
              <button
                type="button"
                aria-pressed={entry.athlete.user_id === selectedId}
                data-testid={`coach-treated-row-${entry.athlete.user_id}`}
                onClick={() => onSelect(entry.athlete.user_id)}
                className="flex min-w-0 flex-1 items-center gap-3 rounded text-left focus-ring-visible"
              >
                <AthleteAvatar userId={entry.athlete.user_id} name={entry.athlete.name} size="sm" muted />
                <span className="min-w-0 flex-1 truncate text-sm text-on-surface-variant">
                  <span className="font-medium text-on-surface">{entry.athlete.name}</span>
                  {' · '}
                  {describeLine(t, language, entry.reason.key, entry.reason.params)}
                </span>
              </button>
              <button
                type="button"
                data-testid={`coach-treated-undo-${entry.athlete.user_id}`}
                onClick={() => onUndo(entry.athlete, entry.reason)}
                className="shrink-0 rounded text-sm font-medium text-primary hover:underline focus-ring-visible"
              >
                {t('coachHome.action.undo')}
              </button>
            </div>
          ))}
        </section>
      )}

      {(calmNames.length > 0 || queue.notSharing.length > 0) && (
        <div className="mx-2 mt-5 flex flex-col gap-2.5 border-t ghost-border pt-4" data-testid="coach-queue-calm">
          {calmNames.length > 0 && (
            <>
              <div className="flex items-center gap-2.5">
                <div aria-hidden="true" className="flex">
                  {calmNames.slice(0, 3).map((athlete, index) => (
                    <span key={athlete.user_id} className={clsx('rounded-full ring-2 ring-surface', index > 0 && '-ml-1.5')}>
                      <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" />
                    </span>
                  ))}
                </div>
                <span className="text-sm font-semibold text-on-surface">{t('coachHome.queue.nothing')}</span>
                <span className="font-mono text-xs text-on-surface-variant">{calmNames.length}</span>
              </div>
              <p className="m-0 text-sm text-on-surface-variant">
                {calmNames.map((athlete, index) => (
                  <span key={athlete.user_id}>
                    {index > 0 && ', '}
                    <button
                      type="button"
                      data-testid={`coach-calm-${athlete.user_id}`}
                      onClick={() => onSelect(athlete.user_id)}
                      className="rounded hover:text-on-surface hover:underline focus-ring-visible"
                    >
                      {athlete.name}
                    </button>
                  </span>
                ))}
              </p>
            </>
          )}
          {queue.notSharing.length > 0 && (
            <p className="m-0 text-xs text-on-surface-variant" data-testid="coach-queue-not-sharing">
              {notSharingLead}
              {queue.notSharing.map((athlete, index) => (
                <span key={athlete.user_id}>
                  {index > 0 && ', '}
                  <button
                    type="button"
                    data-testid={`coach-not-sharing-${athlete.user_id}`}
                    onClick={() => onSelect(athlete.user_id)}
                    className="rounded hover:text-on-surface hover:underline focus-ring-visible"
                  >
                    {athlete.name}
                  </button>
                </span>
              ))}
              {notSharingTail}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

/** Board A: the queue, and the pane of the athlete picked in it. */
export function CoachQueueView({ athletes, selectedId, onSelect, onSeeAll, today, destinations }: CoachQueueViewProps) {
  const { t } = useTranslation();
  const isMobile = useIsMobile();
  const showError = useErrorToast();
  const { dismiss } = useDismissAttention();
  const { restore } = useRestoreAttention();
  const queue = useMemo(() => buildQueue(athletes), [athletes]);

  const selected = useMemo(() => {
    const byId = selectedId === null ? undefined : athletes.find((a) => a.user_id === selectedId);
    if (byId !== undefined) return byId;
    // A wide screen opens on the first of the queue; a phone opens on the list.
    return isMobile ? null : firstQueued(queue);
  }, [athletes, isMobile, queue, selectedId]);

  const entry = useMemo(() => {
    if (selected === null) return null;
    return [...queue.today, ...queue.week].find((e) => e.athlete.user_id === selected.user_id) ?? null;
  }, [queue, selected]);
  const treatedReason = useMemo(() => {
    if (selected === null) return null;
    return queue.treated.find((e) => e.athlete.user_id === selected.user_id)?.reason ?? null;
  }, [queue, selected]);

  const treat = (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
    dismiss({ athlete_user_id: athlete.user_id, reason_id: reason.id, fingerprint: reason.fingerprint }).catch(() =>
      showError(t('coachHome.action.treatFailed')),
    );
  };
  const undo = (athlete: CoachRosterAthlete, reason: CoachAttentionReason) => {
    restore({ athlete_user_id: athlete.user_id, reason_id: reason.id }).catch(() =>
      showError(t('coachHome.action.undoFailed')),
    );
  };

  const list = <QueueList queue={queue} selectedId={selected?.user_id ?? null} onSelect={onSelect} onSeeAll={onSeeAll} onUndo={undo} />;

  const pane =
    selected === null ? (
      <EmptyState className="px-6 py-8" data-testid="coach-pane-empty">
        {t('coachHome.queue.selectAthlete')}
      </EmptyState>
    ) : (
      <CoachAthletePane
        athlete={selected}
        entry={entry}
        treatedReason={treatedReason}
        today={today}
        destinations={destinations}
        onTreat={treat}
        onUndo={undo}
      />
    );

  if (isMobile) {
    if (selected === null || selectedId === null) {
      return <div className="min-h-0 flex-1 overflow-y-auto">{list}</div>;
    }
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        <button
          type="button"
          data-testid="coach-pane-back"
          onClick={() => onSelect(null)}
          className="flex h-11 items-center gap-1 px-3 text-sm font-medium text-primary focus-ring-visible"
        >
          <ChevronLeft className="h-4 w-4" aria-hidden="true" />
          {t('coachHome.queue.title')}
        </button>
        <section aria-label={selected.name} className="px-4 pb-10 pt-2">
          {pane}
        </section>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1">
      <div className="w-[340px] shrink-0 overflow-y-auto border-r ghost-border lg:w-[420px]">{list}</div>
      <section aria-label={selected?.name} className="min-w-0 flex-1 overflow-y-auto px-6 pb-10 pt-6 lg:px-8">
        {pane}
      </section>
    </div>
  );
}
