// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab's training status — today's form band, form as a share of fitness, its trend, the load ratio and recovery days
// ABOUTME: Names what the server computed and derives nothing: a thin history is said in a sentence, never drawn as zeros

import React, { useMemo, useState } from 'react';
import { Text, View, type LayoutChangeEvent } from 'react-native';
import Svg, { Circle, Line, Path } from 'react-native-svg';
import type { FormReading, FormTrendPoint, TrainingStatusResponse } from '@pierre/shared-types';
import {
  FORM_BAND_DETAIL_KEY,
  FORM_BAND_LABEL_KEY,
  TRAINING_STATUS_KEY,
  formLine,
  loadRatioLine,
  recoveryLine,
  signedWhole,
  trendLabelLine,
} from '@pierre/shared-constants';
import { projectFormTrend } from '@pierre/domain-utils';
import { formatDecimal } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { civilShortDate } from './homeFormat';

/** The chart's height, in points. Its width is the column's, measured. */
const TREND_HEIGHT = 64;
/** Room kept clear inside the chart, so the end dot and its ring are not clipped. */
const TREND_PADDING = 7;
const LINE_WIDTH = 2;
const DOT_RADIUS = 4;
const DOT_RING = 2;

interface HomeStatusProps {
  /** The server's answer; null until one arrives. */
  response: TrainingStatusResponse | null;
  isError: boolean;
  onRetry: () => void;
}

/**
 * Form over the trend's days, as one line against the zero line where form is
 * level with fitness, today marked by a dot. Drawn in the column's own width,
 * which the first layout reports; until then the chart's place is held empty.
 */
function FormTrend({ trend }: { trend: readonly FormTrendPoint[] }) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const [width, setWidth] = useState(0);
  const geometry = useMemo(
    () =>
      width > 0
        ? projectFormTrend(
            trend.map((point) => point.pct_of_fitness),
            { width, height: TREND_HEIGHT, padding: TREND_PADDING },
          )
        : null,
    [trend, width],
  );
  const measured = trend.filter((point) => point.pct_of_fitness !== null);

  if (measured.length < 2) {
    return (
      <Text className="mt-3 text-xs text-text-secondary" testID="home-status-trend-short">
        {t(TRAINING_STATUS_KEY.trendTooShort)}
      </Text>
    );
  }
  // The days the line under it runs across, which on a thin history is fewer
  // than the window the server aims for.
  const label = trendLabelLine(t, trend);
  const first = measured[0];
  const last = measured[measured.length - 1];
  const lastPoint = geometry?.points[trend.lastIndexOf(last)] ?? null;
  const onLayout = (event: LayoutChangeEvent) => setWidth(event.nativeEvent.layout.width);

  return (
    <View className="mt-4">
      {label !== null && (
        <Text className="text-xs text-text-secondary" testID="home-status-trend-label">
          {label}
        </Text>
      )}
      <View
        className="mt-2"
        style={{ height: TREND_HEIGHT }}
        onLayout={onLayout}
        accessible
        accessibilityRole="image"
        accessibilityLabel={t(TRAINING_STATUS_KEY.trendAlt, {
          from: civilShortDate(first.date, language),
          to: civilShortDate(last.date, language),
          first: signedWhole(first.pct_of_fitness ?? 0),
          last: signedWhole(last.pct_of_fitness ?? 0),
        })}
        testID="home-status-trend"
      >
        {geometry !== null && (
          <Svg width={width} height={TREND_HEIGHT} viewBox={`0 0 ${width} ${TREND_HEIGHT}`}>
            <Line
              x1={0}
              x2={width}
              y1={geometry.zeroY}
              y2={geometry.zeroY}
              stroke={colors.tokens.outlineVariant}
              strokeWidth={1}
            />
            <Path
              d={geometry.path}
              fill="none"
              stroke={colors.tokens.primary}
              strokeWidth={LINE_WIDTH}
              strokeLinecap="round"
              strokeLinejoin="round"
              testID="home-status-trend-line"
            />
            {lastPoint !== null && (
              <Circle
                cx={lastPoint.x}
                cy={lastPoint.y}
                r={DOT_RADIUS}
                fill={colors.tokens.primary}
                stroke={colors.tokens.surface}
                strokeWidth={DOT_RING}
              />
            )}
          </Svg>
        )}
      </View>
      <View className="mt-1 flex-row justify-between">
        <Text className="font-mono text-xs text-text-secondary">{civilShortDate(trend[0].date, language)}</Text>
        <Text className="font-mono text-xs text-text-secondary">
          {civilShortDate(trend[trend.length - 1].date, language)}
        </Text>
      </View>
      <Text className="mt-2 text-xs text-text-secondary">{t(TRAINING_STATUS_KEY.trendHint)}</Text>
    </View>
  );
}

/** The reading itself: the band, the figure, the trend, then the two lines of load and recovery. */
function StatusReading({ status, form }: { status: TrainingStatusResponse; form: FormReading }) {
  const { t, language } = useTranslation();
  const figure = formLine(t, form);
  return (
    <View className="px-4" testID="home-status-reading">
      <Text className="text-xl font-semibold text-text-primary" testID="home-status-band">
        {t(FORM_BAND_LABEL_KEY[form.band])}
      </Text>
      {figure !== null && (
        <Text className="mt-1 font-mono text-xs text-text-secondary" testID="home-status-form">
          {figure}
        </Text>
      )}
      <Text className="mt-1 text-sm text-text-secondary">{t(FORM_BAND_DETAIL_KEY[form.band])}</Text>
      <FormTrend trend={status.trend} />
      {status.load_ratio !== null && (
        <Text className="mt-3 text-sm text-text-primary" testID="home-status-load">
          {loadRatioLine(t, status.load_ratio, formatDecimal(status.load_ratio.ratio, 1, language))}
        </Text>
      )}
      {status.recovery_days !== null && (
        <Text className="mt-1 text-sm text-text-primary" testID="home-status-recovery">
          {recoveryLine(t, status.recovery_days)}
        </Text>
      )}
    </View>
  );
}

/**
 * The training-status section. A status that could not be read says so and
 * offers a retry; one the server answered without a reading — too little
 * history to stand behind today — is a sentence, with no figure and no chart.
 * Nothing is drawn while the first answer is on its way: the section's title
 * holds its place.
 */
export function HomeStatus({ response, isError, onRetry }: HomeStatusProps) {
  const { t } = useTranslation();
  const form = response?.form ?? null;

  return (
    <Section title={t(TRAINING_STATUS_KEY.heading)} testID="home-status">
      {response === null ? (
        isError && (
          <EmptyState
            action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-status-retry' }}
            testID="home-status-failed"
          >
            {t(TRAINING_STATUS_KEY.loadFailed)}
          </EmptyState>
        )
      ) : form === null ? (
        <EmptyState testID="home-status-empty">{t(TRAINING_STATUS_KEY.empty)}</EmptyState>
      ) : (
        <StatusReading status={response} form={form} />
      )}
    </Section>
  );
}
