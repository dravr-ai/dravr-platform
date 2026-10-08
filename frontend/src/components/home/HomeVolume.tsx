// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's weekly volume — this week's distance, time and climbing, and a bar per week over the last twelve
// ABOUTME: Filterable by sport; sums only what the server stored, and a week before the history begins is not drawn at all

import { useMemo, useState, type PointerEvent } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import type { TrainingVolumeResponse } from '@pierre/shared-types';
import {
  ALL_SPORTS,
  TRAINING_VOLUME_KEY,
  activitiesLine,
  volumeChartLabel,
  weekMinutesSeconds,
} from '@pierre/shared-constants';
import {
  formatDuration,
  metricValue,
  projectVolumeBars,
  volumeBarAt,
  volumeMetric,
  volumeSports,
  weekTotals,
  type SketchBox,
  type VolumeMetric,
  type VolumeTotals,
} from '@pierre/domain-utils';
import { formatDecimal } from '@pierre/chat-utils';
import { formatKilometres } from '@pierre/ui-logic';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { Tabs } from '../ui/Tabs';
import { useTrainingVolume } from '../../hooks/useHome';
import { formatCivilDate, sportLabel } from './homeFormat';

/**
 * The chart's own coordinate space, stretched to the column's width. Bars are
 * rectangles, so the stretch only widens them.
 */
const VOLUME_PADDING = 2;
const VOLUME_BOX: SketchBox = { width: 240, height: 72, padding: VOLUME_PADDING };

/** A week's Monday as the readout and the axis ends print it: "20 Jul". */
const WEEK_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short' };

/** A week's figure for the metric the bars stand for, in the reader's notation. */
function metricText(t: TFunction, metric: VolumeMetric, value: number, language: string): string {
  return metric === 'distance' ? formatKilometres(value, language) : formatDuration(t, weekMinutesSeconds(value));
}

/** This week's three figures and its session count, for the selected sport or all of them. */
function ThisWeek({ totals }: { totals: VolumeTotals }) {
  const { t, language } = useTranslation();
  const figures = [
    { key: 'distance', label: t(TRAINING_VOLUME_KEY.distance), value: formatKilometres(totals.distance_meters, language) },
    { key: 'time', label: t(TRAINING_VOLUME_KEY.time), value: formatDuration(t, weekMinutesSeconds(totals.duration_seconds)) },
    {
      key: 'elevation',
      label: t(TRAINING_VOLUME_KEY.elevation),
      value: `${formatDecimal(Math.round(totals.elevation_gain_meters), 0, language)} m`,
    },
  ];
  return (
    <div data-testid="home-volume-week">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-sm font-medium text-on-surface">{t(TRAINING_VOLUME_KEY.thisWeek)}</span>
        <span data-testid="home-volume-count" className="text-xs text-on-surface-variant">
          {activitiesLine(t, totals.activities)}
        </span>
      </div>
      <dl className="mt-2 grid grid-cols-3 gap-3">
        {figures.map((figure) => (
          <div key={figure.key} className="min-w-0">
            <dt className="text-xs text-on-surface-variant">{figure.label}</dt>
            <dd data-testid={`home-volume-${figure.key}`} className="mt-0.5 truncate font-mono text-base text-on-surface">
              {figure.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}

/**
 * One bar per week the server answered, this week's last and in the primary
 * ink. Pointing at the chart reads the nearest week out under it; at rest the
 * readout is this week's.
 */
function VolumeTrend({ starts, totals }: { starts: readonly string[]; totals: readonly VolumeTotals[] }) {
  const { t, language } = useTranslation();
  const [pointed, setPointed] = useState<number | null>(null);
  const metric = volumeMetric(totals);
  const values = totals.map((week) => metricValue(week, metric));
  const bars = projectVolumeBars(values, VOLUME_BOX);

  if (starts.length < 2) {
    return (
      <p data-testid="home-volume-trend-short" className="mt-4 text-xs text-on-surface-variant">
        {t(TRAINING_VOLUME_KEY.trendTooShort)}
      </p>
    );
  }

  const last = starts.length - 1;
  const shown = pointed ?? last;
  const label = volumeChartLabel(t, metric, starts.length);
  const day = (date: string) => formatCivilDate(date, language, WEEK_DATE);
  const onPointer = (event: PointerEvent<HTMLDivElement>) => {
    const frame = event.currentTarget.getBoundingClientRect();
    if (frame.width <= 0) return;
    setPointed(volumeBarAt(bars.length, ((event.clientX - frame.left) / frame.width) * VOLUME_BOX.width, VOLUME_BOX));
  };

  return (
    <div className="mt-5">
      <p data-testid="home-volume-trend-label" className="text-xs text-on-surface-variant">
        {label}
      </p>
      <div
        className="relative mt-2 h-[72px] w-full touch-pan-y"
        onPointerMove={onPointer}
        onPointerDown={onPointer}
        onPointerLeave={() => setPointed(null)}
      >
        <svg
          viewBox={`0 0 ${VOLUME_BOX.width} ${VOLUME_BOX.height}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={t(TRAINING_VOLUME_KEY.chartAlt, {
            label,
            from: day(starts[0]),
            first: metricText(t, metric, values[0], language),
            last: metricText(t, metric, values[last], language),
          })}
          data-testid="home-volume-trend"
          className="block h-full w-full"
        >
          <line
            x1={0}
            x2={VOLUME_BOX.width}
            y1={VOLUME_BOX.height - VOLUME_PADDING}
            y2={VOLUME_BOX.height - VOLUME_PADDING}
            className="stroke-outline-variant"
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
          />
          {bars.map((bar, index) => (
            <rect
              key={starts[index]}
              data-testid="home-volume-bar"
              x={bar.x}
              y={bar.y}
              width={bar.width}
              height={bar.height}
              rx={1}
              className={index === shown ? 'fill-primary' : 'fill-primary/30'}
            />
          ))}
        </svg>
      </div>
      <div className="mt-1 flex justify-between font-mono text-xs text-on-surface-variant">
        <span>{day(starts[0])}</span>
        <span>{day(starts[last])}</span>
      </div>
      <p data-testid="home-volume-readout" className="mt-2 text-xs text-on-surface-variant">
        {t(TRAINING_VOLUME_KEY.weekReadout, { date: day(starts[shown]), value: metricText(t, metric, values[shown], language) })}
      </p>
    </div>
  );
}

/** The card's reading: the sport filter when there is a choice, this week, then the trend. */
function VolumeReading({ volume }: { volume: TrainingVolumeResponse }) {
  const { t } = useTranslation();
  const sports = useMemo(() => volumeSports(volume.weeks), [volume.weeks]);
  const [chosen, setChosen] = useState<string>(ALL_SPORTS);
  // A sport that dropped out of the window on a re-read falls back to all of them.
  const selected = chosen !== ALL_SPORTS && sports.includes(chosen) ? chosen : ALL_SPORTS;
  const sport = selected === ALL_SPORTS ? null : selected;
  const totals = useMemo(() => volume.weeks.map((week) => weekTotals(week, sport)), [volume.weeks, sport]);
  const starts = useMemo(() => volume.weeks.map((week) => week.week_start), [volume.weeks]);

  return (
    <div data-testid="home-volume-reading">
      {sports.length > 1 && (
        <div className="mb-3 overflow-x-auto" role="group" aria-label={t(TRAINING_VOLUME_KEY.filterLabel)}>
          <Tabs
            size="sm"
            activeTab={selected}
            onChange={setChosen}
            tabs={[
              { id: ALL_SPORTS, label: t(TRAINING_VOLUME_KEY.allSports) },
              ...sports.map((key) => ({ id: key, label: sportLabel(t, key) })),
            ]}
          />
        </div>
      )}
      <ThisWeek totals={totals[totals.length - 1]} />
      <VolumeTrend starts={starts} totals={totals} />
    </div>
  );
}

/**
 * The weekly-volume section. A volume that could not be read says so and
 * offers a retry; an answer with no weeks — nothing stored yet — is a
 * sentence, never a row of empty bars.
 */
export function HomeVolume() {
  const { t } = useTranslation();
  const volume = useTrainingVolume();

  return (
    <Section title={t(TRAINING_VOLUME_KEY.heading)} headingLevel={3} data-testid="home-volume">
      {volume.data === undefined ? (
        volume.isError ? (
          <EmptyState
            data-testid="home-volume-failed"
            action={{ label: t('common.retry'), onClick: () => void volume.refetch() }}
          >
            {t(TRAINING_VOLUME_KEY.loadFailed)}
          </EmptyState>
        ) : (
          <div className="flex py-3" role="status" aria-label={t('common.loading')}>
            <div className="pierre-spinner" />
          </div>
        )
      ) : volume.data.weeks.length === 0 ? (
        <EmptyState data-testid="home-volume-empty">{t(TRAINING_VOLUME_KEY.empty)}</EmptyState>
      ) : (
        <VolumeReading volume={volume.data} />
      )}
    </Section>
  );
}
