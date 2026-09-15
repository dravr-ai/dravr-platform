// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One row of the Discover catalogue — a category-tinted initials glyph, name, one-line description, install count and an ink action
// ABOUTME: 64 tall, full-bleed hairline, no card: replaces the boxed coach card per Boreal v2.2 Phase 5 (D7)

import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { initialsFor } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';
import { coachCategoryLabelKey } from '@pierre/shared-constants';
import { categoryAccent, categoryInk, useThemeColors } from '../../constants/theme';
import type { StoreAgent } from '../../types';

/** The tint alpha behind the initials glyph — the same suffix `InitialsAvatar` uses for its own circle. */
const GLYPH_TINT_ALPHA = '33';

/** The glyph's edge, matching D7's 40 pt discover row. */
const GLYPH_SIZE = 40;

export interface DiscoverRowProps {
  agent: StoreAgent;
  onPress: (agent: StoreAgent) => void;
  /** Feeds `coach-card-${index}`, the id every store Maestro flow and jest test pins. */
  index: number;
}

/**
 * A coach is not a provider brand, so its glyph is not `providerGlyph` — it
 * is the agent's own initials tinted by its pillar category, the same pairing
 * the category badge and the editor's category picker already draw with
 * `categoryAccent`/`categoryInk`.
 */
export function DiscoverRow({ agent, onPress, index }: DiscoverRowProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const accent = categoryAccent(colors, agent.category);
  const ink = categoryInk(colors, agent.category);

  return (
    <Pressable
      testID={`coach-card-${index}`}
      accessible
      accessibilityRole="button"
      className="flex-row items-center min-h-[64px] px-4 bg-background-primary"
      onPress={() => onPress(agent)}
    >
      <View
        style={{
          width: GLYPH_SIZE,
          height: GLYPH_SIZE,
          borderRadius: GLYPH_SIZE / 2,
          backgroundColor: `${accent}${GLYPH_TINT_ALPHA}`,
          alignItems: 'center',
          justifyContent: 'center',
        }}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        <Text style={{ color: ink, fontSize: Math.round(GLYPH_SIZE * 0.4), fontWeight: '700' }}>
          {initialsFor(agent.title)}
        </Text>
      </View>

      {/*
        The column stretches to the row's height so its bottom hairline is the
        row's divider, inset to the text rather than running the full width —
        the same shape `ConversationRow` draws its own hairline in.
      */}
      <View
        className="flex-1 ml-3 self-stretch justify-center border-b border-border-faint"
        style={{ borderBottomWidth: StyleSheet.hairlineWidth }}
      >
        <View className="flex-row items-center">
          <Text
            className="flex-shrink text-base font-medium text-text-primary"
            numberOfLines={1}
            testID={`coach-title-${index}`}
          >
            {agent.title}
          </Text>
          <View
            testID="category-badge"
            accessibilityLabel={t(coachCategoryLabelKey(agent.category))}
            style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: accent, marginLeft: 6 }}
          />
          <Text
            testID="install-count"
            className="text-sm font-mono tabular-nums text-text-tertiary ml-2"
            style={{ marginLeft: 'auto' }}
          >
            {t(agent.install_count === 1 ? 'discover.installCountOne' : 'discover.installCountN', {
              count: agent.install_count,
            })}
          </Text>
        </View>

        {/*
          The install action always shows, whether or not the listing carries
          a description — a row promising an action the athlete cannot see is
          worse than one with a blank line above it.
        */}
        <View className="flex-row items-center mt-0.5">
          {agent.description ? (
            <Text className="flex-1 text-sm text-text-secondary" numberOfLines={1}>
              {agent.description}
            </Text>
          ) : (
            <View className="flex-1" />
          )}
          <Text testID={`install-action-${index}`} className="text-sm font-medium text-primary ml-2">
            {t('discover.install')}
          </Text>
        </View>
      </View>
    </Pressable>
  );
}
