// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Settings › Physiology on the phone — the athlete's anchors per sport family, each with its value, range, source, date and confidence
// ABOUTME: Read-only: the server words each value as the envelope notice does, a low-confidence one as a range or an estimate (carnet#1516)

import React from 'react';
import { ActivityIndicator, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import {
  PHYSIOLOGY_ANCHOR_LABEL_KEY,
  PHYSIOLOGY_KEY,
  SPORT_FAMILY_LABEL_KEY,
} from '@pierre/shared-constants';
import type { PhysiologyAnchor, SportFamily } from '@pierre/shared-types';
import { describeApiError, physiologyAnchorFacts } from '@pierre/ui-logic';
import { useThemeColors } from '../../constants/theme';
import { EmptyState, PaneScrollView, Row, Section } from '../../components/ui';
import { usePhysiology } from '../../hooks/usePhysiology';

/** An anchor's value over who set it, on the trailing side of its row. */
function ValueAndSource({ testID, anchor }: { testID: string; anchor: PhysiologyAnchor }) {
  return (
    <View className="max-w-[55%] items-end py-2">
      <Text className="text-right text-base tabular-nums text-text-primary" testID={`${testID}-value`}>
        {anchor.value_text}
      </Text>
      <Text className="text-right text-sm text-text-secondary" testID={`${testID}-source`}>
        {anchor.source_text}
      </Text>
    </View>
  );
}

/**
 * The anchors the athlete's workouts are set on, one group per sport family.
 * Nothing here is edited: the agent's `set_physiology` is the one writer, and
 * the pane's line says so.
 */
export function PhysiologyScreen() {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  const physiology = usePhysiology();

  const familyRows = (sport: SportFamily, anchors: PhysiologyAnchor[]) =>
    anchors.map((anchor, index) => (
      <Row
        key={anchor.anchor}
        title={t(PHYSIOLOGY_ANCHOR_LABEL_KEY[anchor.anchor])}
        subtitle={physiologyAnchorFacts(t, anchor, language).join(' · ')}
        trailing={<ValueAndSource testID={`physiology-${sport}-${anchor.anchor}`} anchor={anchor} />}
        last={index === anchors.length - 1}
        testID={`physiology-${sport}-${anchor.anchor}`}
      />
    ));

  /** The pane while no family is listed: loading, failed, or nothing stored. */
  const renderState = () => {
    if (physiology.isPending) {
      return (
        <View className="items-start px-4 py-3" accessibilityLabel={t('common.loading')} testID="physiology-loading">
          <ActivityIndicator color={colors.tokens.primary} />
        </View>
      );
    }
    if (physiology.isError) {
      return (
        <View className="flex-row flex-wrap items-baseline px-4" testID="physiology-load-failed">
          <Text className="text-sm text-error" accessibilityRole="alert">
            {describeApiError(physiology.error, { t, fallbackKey: PHYSIOLOGY_KEY.loadFailed })}
          </Text>
          <Text
            className="ml-1 text-sm font-medium text-primary"
            accessibilityRole="button"
            onPress={() => {
              void physiology.refetch();
            }}
            testID="physiology-retry"
          >
            {t('common.retry')}
          </Text>
        </View>
      );
    }
    return <EmptyState testID="physiology-empty">{t(PHYSIOLOGY_KEY.empty)}</EmptyState>;
  };

  const sports = physiology.data?.sports ?? [];

  return (
    <View className="flex-1 bg-background-primary" testID="physiology-screen">
      <PaneScrollView contentContainerClassName="pt-6 pb-8">
        <Text className="px-4 pb-6 text-sm text-text-secondary">{t(PHYSIOLOGY_KEY.blurb)}</Text>
        <View className="gap-8">
          {sports.length === 0 ? (
            <Section title={t(PHYSIOLOGY_KEY.title)} testID="physiology-state">
              {renderState()}
            </Section>
          ) : (
            sports.map((family) => (
              <Section
                key={family.sport}
                title={t(SPORT_FAMILY_LABEL_KEY[family.sport])}
                testID={`physiology-${family.sport}`}
              >
                {familyRows(family.sport, family.anchors)}
              </Section>
            ))
          )}
        </View>
      </PaneScrollView>
    </View>
  );
}
