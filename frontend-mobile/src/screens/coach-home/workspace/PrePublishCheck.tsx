// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's line before publish on the phone — each draft session resting on convention or outside its evidence, not yet the coach's call
// ABOUTME: A section of hairline rows: « C'est mon choix » keeps the session as the coach's call, saved on the day; « Revoir » opens its editor (carnet#1814, #1818)

import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { useTranslation } from '@pierre/i18n';
import { COACH_CALL_KEYS, type PrePublishItem } from '@pierre/chat-utils';
import { PLAN_WORKSPACE_KEYS, ROW_DATE, formatCivilDate } from '@pierre/ui-logic';
import { Section } from '../../../components/ui';

interface PrePublishCheckProps {
  items: PrePublishItem[];
  /** The athlete, as the sentence names them. */
  athleteName: string;
  /** « C'est mon choix »: keep the session as the coach's call. */
  onKeep: (item: PrePublishItem) => void;
  /** A call is being saved: the answers wait. */
  keeping: boolean;
  /** « Revoir »: open the editor on the session. */
  onReview: (item: PrePublishItem) => void;
}

/**
 * What the coach is asked before publishing: a section titled « Avant de
 * publier », what keeping a session as the coach's call means under it, then
 * one hairline row per session whose evidence is coaching convention or that
 * sits outside its family's evidence, with the two answers. A question, not an
 * alert: no box and no tone (DESIGN.md "Phone", "Cards / Containers"). Nothing
 * at all when there is none.
 */
export function PrePublishCheck({ items, athleteName, onKeep, keeping, onReview }: PrePublishCheckProps) {
  const { t, language } = useTranslation();
  if (items.length === 0) return null;
  return (
    <Section title={t(PLAN_WORKSPACE_KEYS.checkTitle)} description={t(COACH_CALL_KEYS.hint)} testID="workspace-prepublish">
      {items.map((item, index) => {
        const key = `${item.day.date}-${item.position}`;
        const last = index === items.length - 1;
        const line = t(item.reason === 'outside' ? PLAN_WORKSPACE_KEYS.checkOutside : PLAN_WORKSPACE_KEYS.checkConvention, {
          session: item.day.workout,
          date: formatCivilDate(item.day.date, language, ROW_DATE),
          name: athleteName,
        });
        return (
          <View key={key} className="px-4" testID={`workspace-check-${key}`}>
            {/* The faint hairline insets to the text, and the last row has none. */}
            <View
              className={`pt-2 ${last ? '' : 'border-b border-border-faint'}`}
              style={last ? undefined : { borderBottomWidth: StyleSheet.hairlineWidth }}
            >
              <Text className="text-sm text-text-primary">{line}</Text>
              <View className="flex-row gap-x-5">
                <Pressable
                  onPress={() => {
                    if (!keeping) onKeep(item);
                  }}
                  accessibilityRole="button"
                  // Each answer is told apart by the line it answers.
                  accessibilityHint={line}
                  accessibilityState={{ disabled: keeping, busy: keeping }}
                  hitSlop={10}
                  className={`min-h-11 justify-center ${keeping ? 'opacity-50' : ''}`}
                  testID={`workspace-check-keep-${key}`}
                >
                  <Text className="text-sm font-medium text-primary">{t(COACH_CALL_KEYS.action)}</Text>
                </Pressable>
                <Pressable
                  onPress={() => onReview(item)}
                  accessibilityRole="button"
                  accessibilityHint={line}
                  hitSlop={10}
                  className="min-h-11 justify-center"
                  testID={`workspace-check-review-${key}`}
                >
                  <Text className="text-sm font-medium text-primary">{t(PLAN_WORKSPACE_KEYS.review)}</Text>
                </Pressable>
              </View>
            </View>
          </View>
        );
      })}
    </Section>
  );
}
