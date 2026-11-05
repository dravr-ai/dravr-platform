// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Board B — Mes athlètes: one row per athlete, sorted by days without activity, never by form, with the non-sharers apart
// ABOUTME: Form reads on each athlete's own scale and CTL is absolute; the table scrolls in its own box, and a phone gets a two-line list

import { clsx } from 'clsx';
import { ChevronLeft } from 'lucide-react';
import type { CoachRosterAthlete, CoachRosterGroup } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';
import {
  agentCell,
  agentCellNeedsLook,
  agentWords,
  buildRosterTable,
  daysToARace,
  formWords,
  lastSessionWords,
  lastWhenWords,
  nonSharerStatusWords,
  planWords,
  providerLetter,
  providerName,
  sourceChipWords,
  sourceChips,
  threadWords,
} from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { EmptyState } from '../ui';
import { AthleteAvatar, FormStrip, StatusDot } from './parts';
import type { CoachDestinations } from './useCoachDestinations';

// Nine columns that fit the ~1110px a 1280px screen leaves beside the rail, the
// gutters and a scrollbar; narrower than that, the table scrolls in its own box.
// A track holds what it was sized for: a longer value takes a second line or an
// ellipsis with its whole text on hover, never the next column. A wider screen
// gives its room to the columns whose words get cut: the name and the agent.
const COLUMNS = 'minmax(160px, 2fr) 170px 44px 140px 84px 130px 160px minmax(120px, 2fr) minmax(96px, 1fr)';
const TABLE_MIN_WIDTH = 'min-w-[1104px]';
// The source track fits three chips, so a fourth folds them into a count.
const MAX_SOURCE_SLOTS = 3;
// Up to two lines of words in a 48px row, the rest behind an ellipsis.
const TWO_LINES = 'line-clamp-2 min-w-0 leading-tight';

interface CoachRosterTableProps {
  athletes: CoachRosterAthlete[];
  groups: CoachRosterGroup[];
  total: number;
  groupId: string | null;
  onGroup: (id: string | null) => void;
  search: string;
  today: string;
  onOpen: (athleteId: string) => void;
  onBack: () => void;
  destinations: CoachDestinations;
}

function useRowWords() {
  const { t, language } = useTranslation();
  return {
    lastWhen: (days: number | null | undefined) => lastWhenWords(t, days),
    formWord: (athlete: CoachRosterAthlete) => formWords(t, language, athlete),
    lastWhat: (athlete: CoachRosterAthlete) => lastSessionWords(t, athlete),
    planWords: (athlete: CoachRosterAthlete) => planWords(t, athlete),
    agentWords: (athlete: CoachRosterAthlete) => agentWords(t, language, athlete),
    threadWords: (athlete: CoachRosterAthlete) => threadWords(t, language, athlete),
  };
}

function GroupTabs({ groups, total, groupId, onGroup }: Pick<CoachRosterTableProps, 'groups' | 'total' | 'groupId' | 'onGroup'>) {
  const { t } = useTranslation();
  const tab = (id: string | null, label: string, count: number) => (
    <button
      key={id ?? 'all'}
      type="button"
      aria-pressed={groupId === id}
      data-testid={`coach-group-tab-${id ?? 'all'}`}
      onClick={() => onGroup(id)}
      className={clsx(
        'flex h-9 shrink-0 items-center gap-1.5 whitespace-nowrap border-b-2 bg-transparent text-sm focus-ring-visible',
        groupId === id ? 'border-primary font-semibold text-on-surface' : 'border-transparent font-medium text-on-surface-variant',
      )}
    >
      {label}
      <span className="font-mono text-xs font-normal text-on-surface-variant">{count}</span>
    </button>
  );
  return (
    <div role="group" aria-label={t('coachHome.roster.groupsAria')} className="flex gap-5 overflow-x-auto border-b ghost-border">
      {tab(null, t('coachHome.roster.allGroups'), total)}
      {groups.map((group) => tab(group.id, group.name, group.athlete_count))}
    </div>
  );
}

function SourceChips({ athlete }: { athlete: CoachRosterAthlete }) {
  const { t, language } = useTranslation();
  const all = sourceChips(athlete);
  // Rows keep one order so a letter can be compared down the column; only a cell
  // too full to show everything leads with the source to reconnect, so that one
  // is never the one folded away.
  const chips = all.length > MAX_SOURCE_SLOTS ? [...all].sort((a, b) => Number(a.state === 'ok') - Number(b.state === 'ok')) : all;
  const shown = chips.length > MAX_SOURCE_SLOTS ? chips.slice(0, MAX_SOURCE_SLOTS - 1) : chips;
  const folded = chips.slice(shown.length);
  const chipClass = 'relative flex h-[22px] min-w-[22px] shrink-0 items-center justify-center rounded-md bg-surface-container-high px-1 text-xs font-semibold text-on-surface';
  const describe = (chip: (typeof chips)[number]) => `${providerName(chip.provider)} — ${sourceChipWords(t, language, chip)}`;
  return (
    <>
      {shown.map((chip) => (
        <span key={chip.provider} role="img" aria-label={describe(chip)} title={providerName(chip.provider)} className={chipClass}>
          {providerLetter(chip.provider)}
          <StatusDot tone={chip.state === 'ok' ? 'ok' : 'bad'} className="absolute -right-[3px] -top-[3px]" />
        </span>
      ))}
      {folded.length > 0 && (
        <span
          role="img"
          aria-label={folded.map(describe).join(', ')}
          title={folded.map((chip) => providerName(chip.provider)).join(', ')}
          className={chipClass}
        >
          +{folded.length}
          {folded.some((chip) => chip.state !== 'ok') && <StatusDot tone="bad" className="absolute -right-[3px] -top-[3px]" />}
        </span>
      )}
    </>
  );
}

function TableRow({ athlete, today, onOpen }: { athlete: CoachRosterAthlete; today: string; onOpen: (id: string) => void }) {
  const { t } = useTranslation();
  const words = useRowWords();
  const training = athlete.training;
  const last = words.lastWhen(training?.days_since_last_activity);
  const form = words.formWord(athlete);
  const plan = words.planWords(athlete);
  const toRace = daysToARace(athlete, today);
  const race = athlete.season?.state === 'a_race_ahead' ? athlete.season.race : null;
  const cell = agentCell(athlete);
  const thread = words.threadWords(athlete);
  const agent = words.agentWords(athlete);
  const cellClass = 'flex min-w-0 items-center gap-1.5 border-b ghost-border-faint px-2';
  return (
    <div role="row" className="grid h-12" style={{ gridTemplateColumns: COLUMNS }} data-testid={`coach-row-${athlete.user_id}`}>
      <div role="rowheader" className={clsx(cellClass, 'pl-0')}>
        <button
          type="button"
          data-testid={`coach-row-open-${athlete.user_id}`}
          onClick={() => onOpen(athlete.user_id)}
          className="flex min-w-0 items-center gap-2.5 rounded text-left focus-ring-visible"
        >
          <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" />
          <span className="flex min-w-0 flex-col">
            <span className="truncate text-sm font-semibold text-on-surface" title={athlete.name}>{athlete.name}</span>
            <span className="truncate text-xs text-on-surface-variant">{athlete.groups.map((g) => g.name).join(' · ')}</span>
          </span>
        </button>
      </div>
      <div role="cell" className={cellClass} title={form.asOf ?? undefined}>
        <FormStrip band={training?.form?.band ?? 'insufficient_history'} size="sm" muted={form.cut} />
        <span className={clsx(TWO_LINES, 'text-sm', form.cut ? 'text-on-surface-variant' : 'text-on-surface')} aria-hidden={form.asOf !== null || undefined}>
          {form.text}
        </span>
        {form.asOf !== null && <span className="sr-only">{form.asOf}</span>}
      </div>
      <div role="cell" className={clsx(cellClass, 'justify-end font-mono text-xs tabular-nums')} data-testid="coach-row-ctl">
        {training?.ctl === null || training?.ctl === undefined ? '—' : training.ctl}
      </div>
      <div role="cell" className={clsx(cellClass, 'whitespace-nowrap')}>
        <span className={clsx('max-w-full shrink-0 truncate', last.recent && 'font-semibold')} title={last.text}>{last.text}</span>
        {words.lastWhat(athlete) !== '' && <span className="truncate text-on-surface-variant" title={words.lastWhat(athlete)}>· {words.lastWhat(athlete)}</span>}
      </div>
      <div role="cell" className={clsx(cellClass, 'gap-1 pr-0')}>
        <SourceChips athlete={athlete} />
      </div>
      <div role="cell" className={cellClass}>
        <span className={TWO_LINES} title={`${plan.phase}${plan.week}`}>
          {plan.phase}
          <span className="text-on-surface-variant">{plan.week}</span>
        </span>
      </div>
      <div role="cell" className={clsx(cellClass, 'whitespace-nowrap')}>
        <span className="truncate" title={race?.name}>{race?.name ?? '—'}</span>
        {toRace !== null && <span className="shrink-0 font-mono text-xs text-on-surface-variant">{t('coachHome.detail.countdown', { days: toRace })}</span>}
      </div>
      <div role="cell" className={cellClass}>
        <StatusDot tone={agentCellNeedsLook(cell) ? 'warn' : cell.kind === 'none' ? 'none' : 'ok'} />
        <span className={TWO_LINES} title={agent}>{agent}</span>
      </div>
      <div role="cell" className={clsx(cellClass, thread.strong ? 'font-semibold text-on-surface' : 'text-on-surface-variant')}>
        <span className={TWO_LINES} title={thread.text}>{thread.text}</span>
      </div>
    </div>
  );
}

function PhoneRow({ athlete, today, onOpen }: { athlete: CoachRosterAthlete; today: string; onOpen: (id: string) => void }) {
  const { t } = useTranslation();
  const words = useRowWords();
  const last = words.lastWhen(athlete.training?.days_since_last_activity);
  const form = words.formWord(athlete);
  const toRace = daysToARace(athlete, today);
  return (
    <button
      type="button"
      data-testid={`coach-row-${athlete.user_id}`}
      onClick={() => onOpen(athlete.user_id)}
      className="flex min-h-[60px] w-full items-center gap-3 border-b ghost-border-faint text-left focus-ring-visible"
    >
      <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="flex items-center gap-2">
          <span className="text-sm font-semibold text-on-surface">{athlete.name}</span>
          {toRace !== null && (
            <span className="ml-auto font-mono text-xs text-on-surface-variant">{t('coachHome.detail.countdown', { days: toRace })}</span>
          )}
        </span>
        <span className="flex min-w-0 items-center gap-2">
          <FormStrip band={athlete.training?.form?.band ?? 'insufficient_history'} size="sm" muted={form.cut} />
          <span className="truncate text-xs text-on-surface-variant" title={form.asOf ?? undefined}>{`${form.text} · ${last.text}`}</span>
        </span>
      </span>
    </button>
  );
}

/** Board B. */
export function CoachRosterTable(props: CoachRosterTableProps) {
  const { athletes, groups, total, groupId, onGroup, search, today, onOpen, onBack, destinations } = props;
  const { t, language } = useTranslation();
  const isMobile = useIsMobile();
  const table = buildRosterTable(athletes, search);
  const empty = table.sharers.length === 0 && table.nonSharers.length === 0;

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-4 overflow-y-auto px-4 pb-10 pt-5 md:px-8 [&>*]:shrink-0" data-testid="coach-roster">
      <div className="flex flex-wrap items-baseline gap-x-3 gap-y-2">
        <button
          type="button"
          data-testid="coach-roster-back"
          aria-label={t('coachHome.roster.backAria')}
          onClick={onBack}
          className="mr-1 inline-flex h-8 items-center gap-1 rounded text-sm font-medium text-primary focus-ring-visible"
        >
          <ChevronLeft className="h-4 w-4" aria-hidden="true" />
          {t('coachHome.queue.title')}
        </button>
        <h1 className="m-0 font-display text-xl font-semibold tracking-tight text-on-surface">{t('coachHome.roster.title')}</h1>
        <span className="font-mono text-xs text-on-surface-variant" data-testid="coach-roster-count">
          {athletes.length}
        </span>
        <span className="text-sm text-on-surface-variant md:ml-auto" data-testid="coach-roster-sort">
          {t('coachHome.roster.sort')}
        </span>
      </div>

      <GroupTabs groups={groups} total={total} groupId={groupId} onGroup={onGroup} />

      {empty && <EmptyState data-testid="coach-roster-empty">{search.trim() === '' ? t('coachHome.noAthletes') : t('coachHome.roster.noMatch')}</EmptyState>}

      {!empty && table.sharers.length > 0 && !isMobile && (
        <div className="min-w-0 overflow-x-auto" data-testid="coach-roster-scroll">
          <div role="table" aria-label={t('coachHome.roster.title')} className={clsx(TABLE_MIN_WIDTH, 'text-sm')}>
            <div role="row" className="grid h-11 text-xs font-medium text-on-surface-variant" style={{ gridTemplateColumns: COLUMNS }}>
              {[
                ['athlete', 'coachHome.roster.col.athlete'],
                ['form', 'coachHome.roster.col.form'],
                ['ctl', 'coachHome.roster.col.ctl'],
                ['last', 'coachHome.roster.col.last'],
                ['sources', 'coachHome.roster.col.sources'],
                ['plan', 'coachHome.roster.col.plan'],
                ['race', 'coachHome.roster.col.race'],
                ['agent', 'coachHome.roster.col.agent'],
                ['thread', 'coachHome.source.plan_thread'],
              ].map(([id, key]) => (
                <div
                  key={id}
                  role="columnheader"
                  aria-sort={id === 'last' ? 'descending' : undefined}
                  className={clsx(
                    'flex items-center border-b ghost-border px-2 leading-tight',
                    id === 'athlete' && 'pl-0',
                    id === 'ctl' && 'justify-end',
                    id === 'last' && 'font-semibold text-on-surface',
                  )}
                >
                  {t(key)}
                  {id === 'last' && <span aria-hidden="true">&nbsp;↓</span>}
                </div>
              ))}
            </div>
            {table.sharers.map((athlete) => (
              <TableRow key={athlete.user_id} athlete={athlete} today={today} onOpen={onOpen} />
            ))}
          </div>
        </div>
      )}

      {!empty && table.sharers.length > 0 && isMobile && (
        <div data-testid="coach-roster-list">
          {table.sharers.map((athlete) => (
            <PhoneRow key={athlete.user_id} athlete={athlete} today={today} onOpen={onOpen} />
          ))}
        </div>
      )}

      {table.nonSharers.length > 0 && (
        <section aria-label={t('coachHome.roster.noShareTitle')} data-testid="coach-roster-non-sharers">
          <div className="pb-2 pt-4 text-xs font-medium text-on-surface-variant">
            {t('coachHome.roster.noShareTitle')} <span className="font-mono">{table.nonSharers.length}</span>
          </div>
          {table.nonSharers.map((athlete) => (
            <div
              key={athlete.user_id}
              data-testid={`coach-nonsharer-${athlete.user_id}`}
              className="flex min-h-12 flex-wrap items-center gap-x-4 gap-y-1 border-b ghost-border-faint py-2"
            >
              <span className="flex w-[210px] shrink-0 items-center gap-2.5">
                <AthleteAvatar userId={athlete.user_id} name={athlete.name} size="sm" muted />
                <span className="flex min-w-0 flex-col">
                  <span className="text-sm font-semibold text-on-surface">{athlete.name}</span>
                  <span className="truncate text-xs text-on-surface-variant">{athlete.groups.map((g) => g.name).join(' · ')}</span>
                </span>
              </span>
              <span className="min-w-0 flex-1 text-sm text-on-surface-variant">{nonSharerStatusWords(t, language, athlete)}</span>
              {athlete.groups.length > 0 && (
                <button
                  type="button"
                  data-testid={`coach-nonsharer-write-${athlete.user_id}`}
                  disabled={destinations.pendingRoomAthleteId !== null}
                  aria-busy={destinations.pendingRoomAthleteId === athlete.user_id}
                  onClick={() => destinations.writeToGroup(athlete, athlete.attention[0] ?? null)}
                  className="rounded text-sm font-medium text-primary hover:underline focus-ring-visible disabled:opacity-60"
                >
                  {destinations.pendingRoomAthleteId === athlete.user_id
                    ? t('coachHome.action.openingGroup')
                    : t('coachHome.action.writeToGroup')}
                </button>
              )}
            </div>
          ))}
        </section>
      )}

      <p className="m-0 text-xs text-on-surface-variant">{t('coachHome.roster.footnote')}</p>
    </div>
  );
}
