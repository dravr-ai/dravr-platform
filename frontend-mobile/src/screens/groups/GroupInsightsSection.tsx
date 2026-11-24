// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders a group's computed weekly report and per-member health flags on mobile
// ABOUTME: Admin-only section gated on the tenant tier flag the digest scheduler sweeps on

import React from 'react';
import { View, Text, ActivityIndicator } from 'react-native';
import { Feather } from '@expo/vector-icons';
import { useThemeColors } from '../../constants/theme';
import { useGroupHealthFlags, useGroupWeeklyReport } from '../../hooks/useGroups';
import type { HealthFlagSeverity, MemberFlag } from '../../types';
import {
  freshMemberLine,
  healthFlagConcern,
  healthFlagDetail,
  weeklyReportRecommendations,
  weeklyReportSummary,
} from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useDistanceUnit } from '@pierre/ui-logic';
import { BADGE_TONE, type BadgeVariant } from '../../components/ui';

/**
 * The badge tone for each severity, read through the badge table so each
 * label takes the ink its tint binds. A warning flag is a state and takes the
 * warning tone, never a pillar, which marks a category and nothing else
 * (DESIGN.md "Badges and chips").
 */
const SEVERITY_TONE: Record<HealthFlagSeverity, BadgeVariant> = {
  info: 'info',
  warning: 'warning',
  critical: 'error',
};

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
 *
 * No card: this mounts inside `GroupInfoSheet`'s Analytics `CollapsibleSection`,
 * under the mono stat row that is the section's real "table of numbers" — this
 * component is prose (the report) and per-member rows (the flags), so it loses
 * its card fill and reads as a continuation of that section rather than a box
 * of its own. The severity badge on each flag row stays a chip (a legitimate
 * small status mark), not a card.
 *
 * The server sends numbers and evidence, never sentences, so every line here
 * is phrased in the reader's language by the shared group formatter. The
 * report's concerns are the health flags themselves, one line per flag.
 *
 * Both cover only the members who share their training with the reader, owner
 * and admins included; the line under the stat row above says so when that is
 * not everyone.
 */
export function GroupInsightsSection({
  groupId,
  isAdmin,
  weeklyDigestEnabled,
}: GroupInsightsSectionProps) {
  const { t, language } = useTranslation();
  const units = useDistanceUnit();
  const colors = useThemeColors();
  const enabled = isAdmin && weeklyDigestEnabled;
  const { report, isLoading: isReportLoading } = useGroupWeeklyReport(groupId, enabled);
  const { flags, isLoading: isFlagsLoading } = useGroupHealthFlags(groupId, enabled);

  if (!isAdmin) {
    return null;
  }

  if (!weeklyDigestEnabled) {
    return (
      <View className="mt-4" testID="group-insights-tier-locked">
        <Text className="text-text-primary text-sm font-semibold">{t('groups.weeklyReport')}</Text>
        <Text className="text-text-tertiary text-xs mt-1">
          {t('app.weeklyReportGated')}
        </Text>
      </View>
    );
  }

  if (isReportLoading || isFlagsLoading) {
    return (
      <View className="mt-4" testID="group-insights-loading">
        <ActivityIndicator size="small" color={colors.pierre.violet} />
      </View>
    );
  }

  const recommendations = report ? weeklyReportRecommendations(t, report.stats) : [];

  return (
    <View className="mt-4" testID="group-insights-section">
      {report && (
        <View className="mb-4">
          <Text className="text-text-primary text-sm font-semibold mb-2">{t('groups.thisWeek')}</Text>
          <Text className="text-text-secondary text-sm" testID="group-report-summary">
            {weeklyReportSummary(t, language, report.stats, units)}
          </Text>

          {report.fresh_members.length > 0 && (
            <View className="mt-3">
              <Text className="text-text-tertiary text-xs font-semibold mb-1">{t('groups.highlights')}</Text>
              {report.fresh_members.map((member) => (
                <View key={member.user_id} className="flex-row items-start mt-1" testID="group-report-highlight">
                  <Feather name="check-circle" size={12} color={colors.success} />
                  <Text className="text-text-secondary text-sm ml-2 flex-1">{freshMemberLine(t, member)}</Text>
                </View>
              ))}
            </View>
          )}

          {flags.length > 0 && (
            <View className="mt-3">
              <Text className="text-text-tertiary text-xs font-semibold mb-1">{t('groups.concerns')}</Text>
              {flags.map((flag) => (
                <View
                  key={`${flag.user_id}-${flag.flag_type}`}
                  className="flex-row items-start mt-1"
                  testID="group-report-concern"
                >
                  <Feather name="alert-triangle" size={12} color={colors.error} />
                  <Text className="text-text-secondary text-sm ml-2 flex-1">{healthFlagConcern(t, flag)}</Text>
                </View>
              ))}
            </View>
          )}

          {recommendations.length > 0 && (
            <View className="mt-3">
              <Text className="text-text-tertiary text-xs font-semibold mb-1">{t('groups.recommendations')}</Text>
              {recommendations.map((recommendation) => (
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

      <View>
        <Text className="text-text-primary text-sm font-semibold mb-2">
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
                <Text className="text-text-tertiary text-xs mt-0.5">{healthFlagDetail(t, flag)}</Text>
              </View>
              <View className={`px-2 py-0.5 rounded ${BADGE_TONE[SEVERITY_TONE[flag.severity]].fill}`}>
                <Text className={`text-xs font-semibold ${BADGE_TONE[SEVERITY_TONE[flag.severity]].ink}`}>
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
