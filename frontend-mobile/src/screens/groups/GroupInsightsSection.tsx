// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders a group's computed weekly report and per-member health flags on mobile
// ABOUTME: Admin-only section gated on the tenant tier flag the digest scheduler sweeps on

import React from 'react';
import { View, Text, ActivityIndicator, type ViewStyle } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useCardStyle, useThemeColors } from '../../constants/theme';
import { useGroupHealthFlags, useGroupWeeklyReport } from '../../hooks/useGroups';
import type { HealthFlagSeverity, MemberFlag } from '../../types';
import { useTranslation } from '@pierre/i18n';

/** Corpus key per flag. Module scope, so the section resolves it at render. */
const FLAG_LABEL_KEYS: Record<MemberFlag, string> = {
  overreaching: 'app.overreaching',
  fresh_form: 'app.freshForm',
  personal_record: 'app.personalRecord',
  deep_fatigue: 'app.deepFatigue',
  inactive: 'app.inactive',
  volume_drop: 'app.volumeDrop',
};

interface GroupInsightsSectionProps {
  groupId: string;
  /** Whether the caller is an admin or owner — the server refuses both routes otherwise. */
  isAdmin: boolean;
  /** The tenant tier flag from `GET /api/groups/permissions`. */
  weeklyDigestEnabled: boolean;
}

/**
 * The weekly report and health flags for one coaching group.
 *
 * Both are computed server-side from the same member fitness snapshots the
 * chat coach sees, and had no caller outside the digest scheduler: an admin
 * could read the numbers only if a digest happened to be delivered to them.
 */
export function GroupInsightsSection({
  groupId,
  isAdmin,
  weeklyDigestEnabled,
}: GroupInsightsSectionProps) {
  const { t } = useTranslation();
  const colors = useThemeColors();
  const sectionCardStyle: ViewStyle = {
    borderRadius: 12,
    ...useCardStyle(),
  };
  const enabled = isAdmin && weeklyDigestEnabled;
  const { report, isLoading: isReportLoading } = useGroupWeeklyReport(groupId, enabled);
  const { flags, isLoading: isFlagsLoading } = useGroupHealthFlags(groupId, enabled);

  /**
   * The badge for each severity: `fill` grounds it as a tint of the hue, `ink`
   * is what the label draws in. A hue set as text on a tint of itself does not
   * clear AA — `warning` measures 4.28:1 that way — so each label takes the ink
   * its hue binds. `info` is a hue of its own, the informational slate, rather
   * than the `outline` text role, which has no bound ink to pair with.
   */
  const severityBadges: Record<HealthFlagSeverity, { fill: string; ink: string }> = {
    info: { fill: colors.info, ink: colors.ink.info },
    warning: { fill: colors.pierre.activity, ink: colors.ink.activity },
    critical: { fill: colors.error, ink: colors.tokens.onErrorContainer },
  };

  if (!isAdmin) {
    return null;
  }

  if (!weeklyDigestEnabled) {
    return (
      <View className="p-4 mb-4" style={sectionCardStyle} testID="group-insights-tier-locked">
        <Text className="text-text-primary text-sm font-semibold">{t('app.weeklyReport')}</Text>
        <Text className="text-text-tertiary text-xs mt-1">
          {t('app.weeklyReportGated')}
        </Text>
      </View>
    );
  }

  if (isReportLoading || isFlagsLoading) {
    return (
      <View className="p-4 mb-4" style={sectionCardStyle} testID="group-insights-loading">
        <ActivityIndicator size="small" color={colors.pierre.violet} />
      </View>
    );
  }

  return (
    <View testID="group-insights-section">
      {report && (
        <View className="p-4 mb-4" style={sectionCardStyle}>
          <Text className="text-text-primary text-base font-bold mb-2">{t('app.thisWeek')}</Text>
          <Text className="text-text-secondary text-sm" testID="group-report-summary">
            {report.summary}
          </Text>

          {report.highlights.length > 0 && (
            <View className="mt-3">
              <Text className="text-text-tertiary text-xs font-semibold mb-1">{t('app.highlights')}</Text>
              {report.highlights.map((highlight) => (
                <View key={highlight} className="flex-row items-start mt-1" testID="group-report-highlight">
                  <Feather name="check-circle" size={12} color={colors.pierre.activity} />
                  <Text className="text-text-secondary text-sm ml-2 flex-1">{highlight}</Text>
                </View>
              ))}
            </View>
          )}

          {report.concerns.length > 0 && (
            <View className="mt-3">
              <Text className="text-text-tertiary text-xs font-semibold mb-1">{t('app.concerns')}</Text>
              {report.concerns.map((concern) => (
                <View key={concern} className="flex-row items-start mt-1" testID="group-report-concern">
                  <Feather name="alert-triangle" size={12} color={colors.error} />
                  <Text className="text-text-secondary text-sm ml-2 flex-1">{concern}</Text>
                </View>
              ))}
            </View>
          )}

          {report.recommendations.length > 0 && (
            <View className="mt-3">
              <Text className="text-text-tertiary text-xs font-semibold mb-1">{t('app.recommendations')}</Text>
              {report.recommendations.map((recommendation) => (
                <View
                  key={recommendation}
                  className="flex-row items-start mt-1"
                  testID="group-report-recommendation"
                >
                  <Feather name="zap" size={12} color={colors.pierre.violet} />
                  <Text className="text-text-secondary text-sm ml-2 flex-1">{recommendation}</Text>
                </View>
              ))}
            </View>
          )}
        </View>
      )}

      <View className="p-4 mb-4" style={sectionCardStyle}>
        <Text className="text-text-primary text-base font-bold mb-2">
          {t('groups.healthFlagsCount', { n: flags.length })}
        </Text>
        {flags.length === 0 ? (
          <Text className="text-text-tertiary text-sm" testID="group-health-flags-empty">
            {t('app.noMemberFlagged')}
          </Text>
        ) : (
          flags.map((flag) => (
            <View
              key={`${flag.user_id}-${flag.flag_type}`}
              className="flex-row items-center py-2"
              testID="group-health-flag-row"
            >
              <View className="flex-1 pr-3">
                <Text className="text-text-primary text-sm font-medium" numberOfLines={1}>
                  {flag.display_name}
                </Text>
                <Text className="text-text-tertiary text-xs mt-0.5">{flag.detail}</Text>
              </View>
              <View
                className="px-2 py-0.5 rounded"
                style={{ backgroundColor: `${severityBadges[flag.severity].fill}20` }}
              >
                <Text
                  className="text-[10px] font-semibold"
                  style={{ color: severityBadges[flag.severity].ink }}
                >
                  {t(FLAG_LABEL_KEYS[flag.flag_type])}
                </Text>
              </View>
            </View>
          ))
        )}
      </View>
    </View>
  );
}
