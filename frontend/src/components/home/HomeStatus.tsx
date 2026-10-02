// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home page's training status — today's form band, form as a share of fitness, its trend, the load ratio and recovery days
// ABOUTME: Names what the server computed and derives nothing: a thin history is said in a sentence, never drawn as zeros

import { useMemo, useState, type PointerEvent } from 'react';
import { useTranslation } from '@pierre/i18n';
import type { TFunction } from '@pierre/i18n';
import type { FormTrendPoint, TrainingStatusResponse } from '@pierre/shared-types';
import {
  FORM_BAND_DETAIL_KEY,
  FORM_BAND_LABEL_KEY,
  TRAINING_STATUS_KEY,
  formLine,
  loadRatioLine,
  recoveryLine,
  signedWhole,
  trendDayLine,
  trendLabelLine,
} from '@pierre/shared-constants';
import { nearestTrendIndex, projectFormTrend, type SketchBox } from '@pierre/domain-utils';
import { formatDecimal } from '@pierre/chat-utils';
import { Section } from '../ui/Section';
import { EmptyState } from '../ui/EmptyState';
import { useTrainingStatus } from '../../hooks/useHome';
import { ROW_DATE, formatCivilDate } from './homeFormat';

/**
 * The chart's own coordinate space. The element is stretched to the column's
 * width, so only the line is drawn in it — with a stroke that does not scale —
 * and the markers are placed over it as percentages of this box.
 */
const TREND_BOX: SketchBox = { width: 320, height: 64, padding: 6 };

/** A day's date as the readout and the axis ends print it: "20 Sep". */
const TREND_DATE: Intl.DateTimeFormatOptions = { day: 'numeric', month: 'short' };

/** A dot over the chart at one projected point, sized in pixels whatever the stretch. */
function Marker({ x, y, className }: { x: number; y: number; className: string }) {
  return (
    <span
      aria-hidden="true"
      className={`pointer-events-none absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full ring-2 ring-surface ${className}`}
      style={{ left: `${(x / TREND_BOX.width) * 100}%`, top: `${(y / TREND_BOX.height) * 100}%` }}
    />
  );
}

/**
 * Form over the trend's days, as one line against the zero line where form is
 * level with fitness. Pointing at the chart reads the nearest day out under
 * it; at rest the readout is the hint that says which side of the line is
 * which.
 */
function FormTrend({ trend }: { trend: readonly FormTrendPoint[] }) {
  const { t, language } = useTranslation();
  const [pointed, setPointed] = useState<number | null>(null);
  const geometry = useMemo(
    () => projectFormTrend(trend.map((point) => point.pct_of_fitness), TREND_BOX),
    [trend],
  );
  if (geometry === null) {
    return (
      <p data-testid="home-status-trend-short" className="mt-3 text-xs text-on-surface-variant">
        {t(TRAINING_STATUS_KEY.trendTooShort)}
      </p>
    );
  }
  // The days the line under it runs across, which on a thin history is fewer
  // than the window the server aims for.
  const label = trendLabelLine(t, trend);
  const measured = trend.filter((point) => point.pct_of_fitness !== null);
  const first = measured[0];
  const last = measured[measured.length - 1];
  const day = (date: string) => formatCivilDate(date, language, TREND_DATE);
  const lastIndex = trend.lastIndexOf(last);
  const lastPoint = geometry.points[lastIndex];
  const pointedPoint = pointed === null ? null : geometry.points[pointed];

  const onPointer = (event: PointerEvent<HTMLDivElement>) => {
    const frame = event.currentTarget.getBoundingClientRect();
    if (frame.width <= 0) return;
    setPointed(
      nearestTrendIndex(geometry.points, ((event.clientX - frame.left) / frame.width) * TREND_BOX.width),
    );
  };

  return (
    <div className="mt-4">
      {label !== null && <p className="text-xs text-on-surface-variant">{label}</p>}
      <div
        className="relative mt-2 h-16 w-full touch-pan-y"
        onPointerMove={onPointer}
        onPointerDown={onPointer}
        onPointerLeave={() => setPointed(null)}
      >
        <svg
          viewBox={`0 0 ${TREND_BOX.width} ${TREND_BOX.height}`}
          preserveAspectRatio="none"
          role="img"
          aria-label={t(TRAINING_STATUS_KEY.trendAlt, {
            from: day(first.date),
            to: day(last.date),
            first: signedWhole(first.pct_of_fitness ?? 0),
            last: signedWhole(last.pct_of_fitness ?? 0),
          })}
          data-testid="home-status-trend"
          className="block h-full w-full"
        >
          <line
            x1={0}
            x2={TREND_BOX.width}
            y1={geometry.zeroY}
            y2={geometry.zeroY}
            className="stroke-outline-variant"
            strokeWidth={1}
            vectorEffect="non-scaling-stroke"
          />
          {pointedPoint && (
            <line
              x1={pointedPoint.x}
              x2={pointedPoint.x}
              y1={0}
              y2={TREND_BOX.height}
              className="stroke-outline"
              strokeWidth={1}
              vectorEffect="non-scaling-stroke"
            />
          )}
          <path
            d={geometry.path}
            fill="none"
            className="stroke-primary"
            strokeWidth={2}
            strokeLinecap="round"
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
        {lastPoint && <Marker x={lastPoint.x} y={lastPoint.y} className="bg-primary" />}
        {pointedPoint && pointed !== lastIndex && (
          <Marker x={pointedPoint.x} y={pointedPoint.y} className="bg-primary" />
        )}
      </div>
      <div className="mt-1 flex justify-between font-mono text-xs text-on-surface-variant">
        <span>{day(trend[0].date)}</span>
        <span>{day(trend[trend.length - 1].date)}</span>
      </div>
      <p data-testid="home-status-trend-readout" className="mt-2 text-xs text-on-surface-variant">
        {pointed === null
          ? t(TRAINING_STATUS_KEY.trendHint)
          : trendDayLine(t, trend[pointed], formatCivilDate(trend[pointed].date, language, ROW_DATE))}
      </p>
    </div>
  );
}

/** The reading itself: the band, the figure, the trend, then the two lines of load and recovery. */
function StatusReading({ status }: { status: TrainingStatusResponse & { form: NonNullable<TrainingStatusResponse['form']> } }) {
  const { t, language } = useTranslation();
  const figure = formLine(t, status.form);
  return (
    <div data-testid="home-status-reading">
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <span data-testid="home-status-band" className="font-display text-lg font-semibold text-on-surface">
          {t(FORM_BAND_LABEL_KEY[status.form.band])}
        </span>
        {figure !== null && (
          <span data-testid="home-status-form" className="text-sm text-on-surface-variant">
            {figure}
          </span>
        )}
      </div>
      <p className="mt-1 text-sm text-on-surface-variant">{t(FORM_BAND_DETAIL_KEY[status.form.band])}</p>
      <FormTrend trend={status.trend} />
      <StatusFacts t={t} language={language} status={status} />
    </div>
  );
}

/** Load against the athlete's own average, and the lighter days form calls for — each only when the server has it. */
function StatusFacts({
  t,
  language,
  status,
}: {
  t: TFunction;
  language: string;
  status: TrainingStatusResponse;
}) {
  if (status.load_ratio === null && status.recovery_days === null) return null;
  return (
    <ul className="mt-3 space-y-1 text-sm text-on-surface">
      {status.load_ratio !== null && (
        <li data-testid="home-status-load">
          {loadRatioLine(t, status.load_ratio, formatDecimal(status.load_ratio.ratio, 1, language))}
        </li>
      )}
      {status.recovery_days !== null && (
        <li data-testid="home-status-recovery">{recoveryLine(t, status.recovery_days)}</li>
      )}
    </ul>
  );
}

/**
 * The training-status section. A status that could not be read says so and
 * offers a retry; one the server answered without a reading — too little
 * history to stand behind today — is a sentence, with no figure and no chart.
 */
export function HomeStatus() {
  const { t } = useTranslation();
  const status = useTrainingStatus();
  const form = status.data?.form ?? null;

  return (
    <Section title={t(TRAINING_STATUS_KEY.heading)} headingLevel={3} data-testid="home-status">
      {status.data === undefined ? (
        status.isError ? (
          <EmptyState
            data-testid="home-status-failed"
            action={{ label: t('common.retry'), onClick: () => void status.refetch() }}
          >
            {t(TRAINING_STATUS_KEY.loadFailed)}
          </EmptyState>
        ) : (
          <div className="flex py-3" role="status" aria-label={t('common.loading')}>
            <div className="pierre-spinner" />
          </div>
        )
      ) : form === null ? (
        <EmptyState data-testid="home-status-empty">{t(TRAINING_STATUS_KEY.empty)}</EmptyState>
      ) : (
        <StatusReading status={{ ...status.data, form }} />
      )}
    </Section>
  );
}
