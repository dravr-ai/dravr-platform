// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace's line before publish on the phone — each draft session resting on convention or outside its evidence, not yet the coach's call
// ABOUTME: « C'est mon choix » keeps it as the coach's call, saved on the day; « Revoir » opens its editor (carnet#1814, #1818)

import React from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useTranslation } from '@pierre/i18n';
import { COACH_CALL_KEYS, type PrePublishItem } from '@pierre/chat-utils';
import { PLAN_WORKSPACE_KEYS, ROW_DATE, formatCivilDate } from '@pierre/ui-logic';
import { useThemeColors } from '../../../constants/theme';

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
 * What the coach is asked before publishing: one line per session whose
 * evidence is coaching convention or that sits outside its family's
 * evidence, with the two answers. Nothing at all when there is none — the
 * check is a question, never a standing banner.
 */
export function PrePublishCheck({ items, athleteName, onKeep, keeping, onReview }: PrePublishCheckProps) {
  const { t, language } = useTranslation();
  const colors = useThemeColors();
  if (items.length === 0) return null;
  return (
    <View className="mx-4 rounded-xl border border-border-faint px-4 py-3" testID="workspace-prepublish">
      <View className="flex-row items-center gap-2">
        <Feather name="alert-triangle" size={16} color={colors.ink.warning} />
        <Text accessibilityRole="header" className="text-sm font-semibold text-text-primary">
          {t(PLAN_WORKSPACE_KEYS.checkTitle)}
        </Text>
      </View>
      {items.map((item, index) => {
        const key = `${item.day.date}-${item.position}`;
        const line = t(item.reason === 'outside' ? PLAN_WORKSPACE_KEYS.checkOutside : PLAN_WORKSPACE_KEYS.checkConvention, {
          session: item.day.workout,
          date: formatCivilDate(item.day.date, language, ROW_DATE),
          name: athleteName,
        });
        return (
          <View
            key={key}
            className={`py-2 ${index > 0 ? 'border-t border-border-faint' : ''}`}
            style={index > 0 ? { borderTopWidth: StyleSheet.hairlineWidth } : undefined}
            testID={`workspace-check-${key}`}
          >
            <Text className="text-sm text-text-primary">{line}</Text>
            <View className="mt-1 flex-row gap-x-5">
              <Pressable
                onPress={() => {
                  if (!keeping) onKeep(item);
                }}
                accessibilityRole="button"
                // Each answer is told apart by the line it answers, and what
                // keeping it means is read with « C'est mon choix ».
                accessibilityHint={`${line} ${t(COACH_CALL_KEYS.hint)}`}
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
        );
      })}
    </View>
  );
}
