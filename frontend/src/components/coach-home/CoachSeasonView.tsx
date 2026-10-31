// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Board E — La saison: every athlete's phases against one time axis, A races ahead first, then those with none and why
// ABOUTME: Compares dates and phases only — never form or load; a phone gets a countdown list from today to the A race

import { useMemo, useState } from 'react';
import { clsx } from 'clsx';
import type { CoachRosterAthlete } from '@pierre/shared-types';
import { COACH_DAY_UNIT_KEYS, COACH_PHASE_LABEL_KEY, COACH_SEASON_STATE_KEY, countKey } from '@pierre/shared-constants';
import { formatList } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import {
  buildSeason,
  civilShortDate,
  countdownBars,
  formatCivilDate,
  nextPhase,
  type SeasonRow,
  type SeasonScale,
} from '@pierre/ui-logic';
import { useIsMobile } from '../../hooks/useBreakpoint';
import { EmptyState } from '../ui';
import { AthleteAvatar, StatusDot } from './parts';
import { phaseClass } from './phaseStyle';
import { PhaseBand } from './PhaseBand';

interface CoachSeasonViewProps {
  athletes: CoachRosterAthlete[];
  today: string;
  onOpen: (athleteId: string) => void;
}

const LEGEND = ['base', 'build', 'specialty', 'taper', 'draft'] as const;

function pct(at: number, span: number): string {
  return `${Math.max(0, Math.min(100, (at / span) * 100)).toFixed(2)}%`;
}

/** The line under an athlete's name: their A race and its countdown, or the state their season is in. */
function useRowSubline() {
  const { t, language } = useTranslation();
  return (row: SeasonRow): string => {
    if (row.state === 'a_race_ahead') {
      const toRace = row.daysToRace === null ? '' : ` ${t('coachHome.detail.countdown', { days: row.daysToRace })}`;
      return `${row.raceName ?? ''}${toRace}`;
    }
    const date =
      row.state === 'nothing_since_last_race'
        ? row.raceDate
        : row.state === 'plan_ended'
          ? (row.athlete.season?.ends ?? null)
          : null;
    return t(COACH_SEASON_STATE_KEY[row.state], date === null ? {} : { date: civilShortDate(date, language) });
  };
}

function DesktopRows({
  rows,
  axis,
  onOpen,
}: {
  rows: SeasonRow[];
  axis: ReturnType<typeof buildSeason>['axis'];
  onOpen: (id: string) => void;
}) {
  const { t, language } = useTranslation();
  const subline = useRowSubline();
  const { span } = axis;
  return (
    <>
      {rows.map((row) => {
        const phaseName = (kind: SeasonRow['bars'][number]['kind']) => t(COACH_PHASE_LABEL_KEY[kind]);
        const end = Math.max(0, ...row.bars.map((b) => b.to), row.raceAt ?? 0);
        const note = row.noDataSince === null ? null : t('coachHome.seasonView.noData', { date: civilShortDate(row.noDataSince, language) });
        return (
          <div
            key={row.athlete.user_id}
            role="row"
            data-testid={`coach-season-row-${row.athlete.user_id}`}
            className="grid min-h-[52px] border-b ghost-border-faint"
            style={{ gridTemplateColumns: '260px minmax(0, 1fr)' }}
          >
            <div role="rowheader" className="flex min-w-0 items-center gap-2.5 py-1.5 pr-3">
              <AthleteAvatar userId={row.athlete.user_id} name={row.athlete.name} size="sm" />
              <span className="flex min-w-0 flex-col">
                <button
                  type="button"
                  data-testid={`coach-season-open-${row.athlete.user_id}`}
                  onClick={() => onOpen(row.athlete.user_id)}
                  className="whitespace-nowrap rounded text-left text-sm font-semibold text-on-surface hover:underline focus-ring-visible"
                >
                  {row.athlete.name}
                </button>
                <span className="truncate text-xs text-on-surface-variant" data-testid="coach-season-subline">
                  {subline(row)}
                </span>
              </span>
            </div>
            <div role="cell" aria-label={`${row.athlete.name} — ${subline(row)}`} className="relative min-h-[52px]">
              {axis.months.map((m) => (
                <span key={m.date} aria-hidden="true" className="absolute inset-y-0 w-px bg-outline-variant/30" style={{ left: pct(m.at, span) }} />
              ))}
              {row.bars.map((bar, index) => (
                <PhaseBand
                  key={`${bar.kind}-${index}`}
                  data-testid="coach-season-bar"
                  data-kind={bar.kind}
                  label={phaseName(bar.kind)}
                  endClear={row.raceAt !== null && Math.abs(bar.to - row.raceAt) <= 1}
                  className={clsx('absolute top-[13px] box-border h-[26px] rounded-md text-xs font-medium', phaseClass(bar.kind))}
                  style={{ left: pct(bar.from, span), width: pct(bar.to - bar.from, span) }}
                />
              ))}
              {row.raceAt !== null && (
                <span
                  aria-hidden="true"
                  data-testid="coach-season-race"
                  className={clsx(
                    'absolute top-[21px] -ml-[5px] box-border h-2.5 w-2.5 rotate-45 border-[1.5px] border-on-surface',
                    row.racePast ? 'bg-surface' : 'bg-on-surface',
                  )}
                  style={{ left: pct(row.raceAt, span) }}
                />
              )}
              {note !== null && (
                <span
                  className="absolute top-[17px] inline-flex items-center gap-1.5 whitespace-nowrap text-xs text-on-surface-variant"
                  style={{ left: pct(Math.min(end + 2, span - 1), span) }}
                >
                  <StatusDot tone="warn" />
                  {note}
                </span>
              )}
              <span
                aria-hidden="true"
                className="absolute inset-y-0 -ml-px w-0.5 bg-on-surface"
                style={{ left: pct(axis.todayAt, span) }}
              />
            </div>
          </div>
        );
      })}
    </>
  );
}

function PhoneRows({ rows, today, onOpen }: { rows: SeasonRow[]; today: string; onOpen: (id: string) => void }) {
  const { t, language } = useTranslation();
  const subline = useRowSubline();
  return (
    <>
      {rows.map((row) => {
        const bars = countdownBars(row.athlete, today);
        const next = nextPhase(row.athlete, today);
        const plan = row.athlete.plan;
        let note = '';
        if (row.noDataSince !== null) {
          note = t('coachHome.seasonView.noData', { date: civilShortDate(row.noDataSince, language) });
        }
        if (note === '' && next !== null) {
          note = t('coachHome.seasonView.phaseIn', {
            phase: t(COACH_PHASE_LABEL_KEY[next.kind]),
            days: t(countKey(next.inDays, language, COACH_DAY_UNIT_KEYS), { count: next.inDays }),
          });
        } else if (note === '' && plan?.phase) {
          note = `${t(COACH_PHASE_LABEL_KEY[plan.phase])} · ${t('coachHome.detail.planWeek', { week: plan.week ?? 1, total: plan.weeks_total })}`;
        }
        return (
          <button
            key={row.athlete.user_id}
            type="button"
            data-testid={`coach-season-row-${row.athlete.user_id}`}
            onClick={() => onOpen(row.athlete.user_id)}
            className="flex w-full flex-col gap-1.5 border-b ghost-border-faint py-2.5 text-left focus-ring-visible"
          >
            <span className="flex items-baseline gap-2">
              <span className="text-sm font-semibold text-on-surface">{row.athlete.name}</span>
              <span className="truncate text-xs text-on-surface-variant">{subline(row)}</span>
            </span>
            {bars.length > 0 && (
              <span aria-hidden="true" className="flex h-2.5 gap-0.5">
                {bars.map((bar, index) => (
                  <span key={`${bar.kind}-${index}`} className={clsx('box-border rounded-[3px]', phaseClass(bar.kind))} style={{ flex: `${bar.days} 1 0` }} />
                ))}
              </span>
            )}
            {note !== '' && (
              <span className="inline-flex items-center gap-1.5 text-xs text-on-surface-variant">
                {row.noDataSince !== null && <StatusDot tone="warn" />}
                {note}
              </span>
            )}
          </button>
        );
      })}
    </>
  );
}

/** Board E. */
export function CoachSeasonView({ athletes, today, onOpen }: CoachSeasonViewProps) {
  const { t, language } = useTranslation();
  const isMobile = useIsMobile();
  const [scale, setScale] = useState<SeasonScale>('season');
  const view = useMemo(() => buildSeason(athletes, today, scale), [athletes, today, scale]);
  const { axis } = view;
  const nothing = view.ahead.length === 0 && view.without.length === 0;

  const range = useMemo(() => {
    const start = formatCivilDate(axis.start, language, { month: 'short', year: 'numeric' });
    const last = axis.months[axis.months.length - 1]?.date ?? axis.start;
    return t('coachHome.seasonView.range', { from: start, to: formatCivilDate(last, language, { month: 'short', year: 'numeric' }) });
  }, [axis, language, t]);

  const section = (title: string, rows: SeasonRow[], testid: string) => (
    <div role={isMobile ? 'group' : 'rowgroup'} aria-label={title} data-testid={testid}>
      <div aria-hidden="true" className="pb-1.5 pt-4 text-xs font-medium text-on-surface-variant">
        {title} <span className="font-mono">{rows.length}</span>
      </div>
      {isMobile ? <PhoneRows rows={rows} today={today} onOpen={onOpen} /> : <DesktopRows rows={rows} axis={axis} onOpen={onOpen} />}
    </div>
  );

  const hiddenLine =
    view.hidden.length === 0
      ? null
      : t('coachHome.seasonView.noShare', { names: formatList(view.hidden.map((a) => a.name), language) });

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3.5 overflow-y-auto px-4 pb-10 pt-5 md:px-8 [&>*]:shrink-0" data-testid="coach-season">
      <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <h1 className="m-0 font-display text-xl font-semibold tracking-tight text-on-surface">
          {t('coachHome.seasonView.title')}
        </h1>
        {!isMobile && <span className="font-mono text-xs text-on-surface-variant">{range}</span>}
        {!isMobile && (
          <div role="group" aria-label={t('coachHome.seasonView.scaleAria')} data-testid="coach-season-scale" className="flex rounded-[10px] bg-surface-container-high p-0.5">
            {(['12w', 'season'] as const).map((id) => (
              <button
                key={id}
                type="button"
                aria-pressed={scale === id}
                data-testid={`coach-season-scale-${id}`}
                onClick={() => setScale(id)}
                className={clsx(
                  'h-7 rounded-lg px-2.5 text-sm focus-ring-visible',
                  scale === id ? 'bg-surface-container-lowest font-semibold text-on-surface ring-1 ring-outline-variant' : 'font-medium text-on-surface-variant',
                )}
              >
                {id === '12w' ? t('coachHome.seasonView.scale12') : t('coachHome.seasonView.scaleFull')}
              </button>
            ))}
          </div>
        )}
        {!isMobile && (
          <div className="flex flex-wrap items-center gap-x-3.5 gap-y-1.5 text-xs text-on-surface-variant md:ml-auto" aria-hidden="true">
            {LEGEND.map((kind) => (
              <span key={kind} className="inline-flex items-center gap-1.5">
                <span className={clsx('box-border h-3 w-[18px] rounded-[3px]', phaseClass(kind))} />
                {t(COACH_PHASE_LABEL_KEY[kind])}
              </span>
            ))}
            <span className="inline-flex items-center gap-1.5">
              <span className="h-[9px] w-[9px] rotate-45 bg-on-surface" />
              {t('coachHome.seasonView.raceA')}
            </span>
          </div>
        )}
      </div>

      {isMobile && <p className="m-0 text-xs font-medium text-on-surface-variant">{t('coachHome.seasonView.ahead')}</p>}

      {nothing && <EmptyState data-testid="coach-season-empty">{t('coachHome.seasonView.empty')}</EmptyState>}

      {!nothing && (
        <div className="min-w-0 overflow-x-auto" data-testid="coach-season-scroll">
          <div
            role={isMobile ? undefined : 'table'}
            aria-label={isMobile ? undefined : t('coachHome.seasonView.tableAria')}
            className={clsx(!isMobile && 'min-w-[1120px]')}
          >
            {!isMobile && (
              <div role="row" className="grid h-10 border-b ghost-border" style={{ gridTemplateColumns: '260px minmax(0, 1fr)' }}>
                <div role="columnheader" className="flex items-center text-xs font-medium text-on-surface-variant">
                  {t('coachHome.seasonView.athleteHeader')}
                </div>
                <div role="columnheader" className="relative">
                  <span className="sr-only">{range}</span>
                  {axis.months.map((m) => (
                    <span
                      key={m.date}
                      aria-hidden="true"
                      className="absolute inset-y-0 flex items-end border-l ghost-border-faint pb-1.5 pl-1.5 text-xs font-medium text-on-surface-variant"
                      style={{ left: pct(m.at, axis.span) }}
                    >
                      {formatCivilDate(m.date, language, { month: 'short' })}
                    </span>
                  ))}
                  <span
                    data-testid="coach-season-today"
                    className="absolute top-0.5 flex h-5 -translate-x-1/2 items-center rounded-full bg-on-surface px-1.5 text-xs font-medium text-surface"
                    style={{ left: pct(axis.todayAt, axis.span) }}
                  >
                    {t('coachHome.seasonView.today')}
                  </span>
                </div>
              </div>
            )}
            {view.ahead.length > 0 && section(t('coachHome.seasonView.sectionRace'), view.ahead, 'coach-season-ahead')}
            {view.without.length > 0 && section(t('coachHome.seasonView.sectionNone'), view.without, 'coach-season-without')}
          </div>
        </div>
      )}

      {hiddenLine !== null && (
        <p className="m-0 text-sm text-on-surface-variant" data-testid="coach-season-hidden">
          {hiddenLine}
        </p>
      )}
    </div>
  );
}
