// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab's weekly volume — this week's distance, time and climbing, and a bar per week over the last twelve
// ABOUTME: Filterable by sport; sums only what the server stored, and a week before the history begins is not drawn at all

import React, { useMemo, useState } from 'react';
import { Pressable, Text, View, type GestureResponderEvent, type LayoutChangeEvent } from 'react-native';
import Svg, { Line, Rect } from 'react-native-svg';
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
  type VolumeMetric,
  type VolumeTotals,
} from '@pierre/domain-utils';
import { formatElevation, type DistanceUnit } from '@pierre/chat-utils';
import { useDistanceUnit } from '@pierre/ui-logic';
import { useTranslation } from '@pierre/i18n';
import { EmptyState, Section, TextTabs } from '../../components/ui';
import { useThemeColors } from '../../constants/theme';
import { civilShortDate, rowDistance, sportLabel, type Translate } from './homeFormat';

/** The chart's height, in points. Its width is the column's, measured. */
const CHART_HEIGHT = 72;
/** Room kept clear inside the chart, so the baseline is not clipped. */
const CHART_PADDING = 2;
const BAR_RADIUS = 1;
/** The weeks other than the one read out, in the primary ink at this opacity. */
const QUIET_BAR_OPACITY = 0.3;

interface HomeVolumeProps {
  /** The server's answer; null until one arrives. */
  response: TrainingVolumeResponse | null;
  isError: boolean;
  onRetry: () => void;
}

/** A week's figure for the metric the bars stand for, in the reader's notation and units. */
function metricText(t: Translate, metric: VolumeMetric, value: number, language: string, unit: DistanceUnit): string {
  return metric === 'distance' ? rowDistance(value, unit, language) : formatDuration(t, weekMinutesSeconds(value));
}

/** This week's three figures and its session count, for the selected sport or all of them. */
function ThisWeek({ totals }: { totals: VolumeTotals }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  const figures = [
    { key: 'distance', label: t(TRAINING_VOLUME_KEY.distance), value: rowDistance(totals.distance_meters, unit, language) },
    { key: 'time', label: t(TRAINING_VOLUME_KEY.time), value: formatDuration(t, weekMinutesSeconds(totals.duration_seconds)) },
    {
      key: 'elevation',
      label: t(TRAINING_VOLUME_KEY.elevation),
      value: formatElevation(totals.elevation_gain_meters, unit, language),
    },
  ];
  return (
    <View className="px-4" testID="home-volume-week">
      <View className="flex-row items-baseline justify-between">
        <Text className="text-sm font-medium text-text-primary">{t(TRAINING_VOLUME_KEY.thisWeek)}</Text>
        <Text className="text-xs text-text-secondary" testID="home-volume-count">
          {activitiesLine(t, totals.activities)}
        </Text>
      </View>
      <View className="mt-2 flex-row gap-3">
        {figures.map((figure) => (
          <View key={figure.key} className="flex-1">
            <Text className="text-xs text-text-secondary">{figure.label}</Text>
            <Text
              className="mt-0.5 font-mono text-base text-text-primary"
              numberOfLines={1}
              testID={`home-volume-${figure.key}`}
            >
              {figure.value}
            </Text>
          </View>
        ))}
      </View>
    </View>
  );
}

/**
 * One bar per week the server answered, drawn in the column's measured
 * width. A tap reads the week under it out; at rest the readout is this
 * week's, whose bar is the one in full ink.
 */
function VolumeTrend({ starts, totals }: { starts: readonly string[]; totals: readonly VolumeTotals[] }) {
  const { t, language } = useTranslation();
  const unit = useDistanceUnit();
  const colors = useThemeColors();
  const [width, setWidth] = useState(0);
  const [pointed, setPointed] = useState<number | null>(null);
  const metric = volumeMetric(totals);
  const values = totals.map((week) => metricValue(week, metric));
  const box = { width, height: CHART_HEIGHT, padding: CHART_PADDING };
  const bars = width > 0 ? projectVolumeBars(values, box) : [];

  if (starts.length < 2) {
    return (
      <Text className="mt-4 px-4 text-xs text-text-secondary" testID="home-volume-trend-short">
        {t(TRAINING_VOLUME_KEY.trendTooShort)}
      </Text>
    );
  }

  const last = starts.length - 1;
  const shown = pointed ?? last;
  const label = volumeChartLabel(t, metric, starts.length);
  const onLayout = (event: LayoutChangeEvent) => setWidth(event.nativeEvent.layout.width);
  const onPress = (event: GestureResponderEvent) => setPointed(volumeBarAt(starts.length, event.nativeEvent.locationX, box));

  return (
    <View className="mt-5 px-4">
      <Text className="text-xs text-text-secondary" testID="home-volume-trend-label">
        {label}
      </Text>
      <Pressable
        className="mt-2"
        style={{ height: CHART_HEIGHT }}
        onLayout={onLayout}
        onPress={onPress}
        accessible
        accessibilityRole="image"
        accessibilityLabel={t(TRAINING_VOLUME_KEY.chartAlt, {
          label,
          from: civilShortDate(starts[0], language),
          first: metricText(t, metric, values[0], language, unit),
          last: metricText(t, metric, values[last], language, unit),
        })}
        testID="home-volume-trend"
      >
        {width > 0 && (
          <Svg width={width} height={CHART_HEIGHT} viewBox={`0 0 ${width} ${CHART_HEIGHT}`}>
            <Line
              x1={0}
              x2={width}
              y1={CHART_HEIGHT - CHART_PADDING}
              y2={CHART_HEIGHT - CHART_PADDING}
              stroke={colors.tokens.outlineVariant}
              strokeWidth={1}
            />
            {bars.map((bar, index) => (
              <Rect
                key={starts[index]}
                x={bar.x}
                y={bar.y}
                width={bar.width}
                height={bar.height}
                rx={BAR_RADIUS}
                fill={colors.tokens.primary}
                fillOpacity={index === shown ? 1 : QUIET_BAR_OPACITY}
                testID="home-volume-bar"
              />
            ))}
          </Svg>
        )}
      </Pressable>
      <View className="mt-1 flex-row justify-between">
        <Text className="font-mono text-xs text-text-secondary">{civilShortDate(starts[0], language)}</Text>
        <Text className="font-mono text-xs text-text-secondary">{civilShortDate(starts[last], language)}</Text>
      </View>
      <Text className="mt-2 text-xs text-text-secondary" testID="home-volume-readout">
        {t(TRAINING_VOLUME_KEY.weekReadout, {
          date: civilShortDate(starts[shown], language),
          value: metricText(t, metric, values[shown], language, unit),
        })}
      </Text>
    </View>
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
    <View testID="home-volume-reading">
      {sports.length > 1 && (
        <TextTabs
          className="mb-3"
          value={selected}
          onChange={setChosen}
          items={[
            { key: ALL_SPORTS, label: t(TRAINING_VOLUME_KEY.allSports) },
            ...sports.map((key) => ({ key, label: sportLabel(t, key) })),
          ]}
          testID="home-volume-filter"
        />
      )}
      <ThisWeek totals={totals[totals.length - 1]} />
      <VolumeTrend starts={starts} totals={totals} />
    </View>
  );
}

/**
 * The weekly-volume section. A volume that could not be read says so and
 * offers a retry; an answer with no weeks — nothing stored yet — is a
 * sentence, never a row of empty bars. Nothing is drawn while the first
 * answer is on its way: the section's title holds its place.
 */
export function HomeVolume({ response, isError, onRetry }: HomeVolumeProps) {
  const { t } = useTranslation();

  return (
    <Section title={t(TRAINING_VOLUME_KEY.heading)} testID="home-volume">
      {response === null ? (
        isError && (
          <EmptyState
            action={{ label: t('common.retry'), onPress: onRetry, testID: 'home-volume-retry' }}
            testID="home-volume-failed"
          >
            {t(TRAINING_VOLUME_KEY.loadFailed)}
          </EmptyState>
        )
      ) : response.weeks.length === 0 ? (
        <EmptyState testID="home-volume-empty">{t(TRAINING_VOLUME_KEY.empty)}</EmptyState>
      ) : (
        <VolumeReading volume={response} />
      )}
    </Section>
  );
}
