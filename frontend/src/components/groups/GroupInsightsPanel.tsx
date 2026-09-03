// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Renders a group's computed weekly report and per-member health flags
// ABOUTME: Admin-only panel gated on the tenant tier flag the digest scheduler sweeps on

import { AlertTriangle, CalendarRange, CheckCircle2, Info, Lightbulb } from 'lucide-react';
import { Card } from '../ui';
import { useGroupHealthFlags, useGroupWeeklyReport } from '../../hooks/useGroups';
import type { HealthFlagSeverity, MemberFlag } from '@pierre/shared-types';
import { useTranslation } from '@pierre/i18n';

interface GroupInsightsPanelProps {
  groupId: string;
  /** Whether the caller is an admin or owner — the server refuses both routes otherwise. */
  isAdmin: boolean;
  /** The tenant tier flag from `GET /api/groups/permissions`. */
  weeklyDigestEnabled: boolean;
}

const FLAG_LABEL_KEYS: Record<MemberFlag, string> = {
  overreaching: 'groups.flagOverreaching',
  fresh_form: 'groups.flagFreshForm',
  personal_record: 'groups.flagPersonalRecord',
  deep_fatigue: 'groups.flagDeepFatigue',
  inactive: 'groups.inactive',
  volume_drop: 'groups.flagVolumeDrop',
};

const SEVERITY_STYLES: Record<HealthFlagSeverity, string> = {
  info: 'bg-surface-container-high/30 text-on-surface-variant',
  warning: 'bg-warning/20 text-on-warning-container',
  critical: 'bg-error/20 text-error',
};

/**
 * The weekly report and health flags for one coaching group.
 *
 * Both are computed server-side on every request from the same member fitness
 * snapshots the chat coach sees, and until now neither had a caller outside
 * the digest scheduler: an admin could read the numbers only if a digest
 * happened to be delivered to them.
 */
export default function GroupInsightsPanel({
  groupId,
  isAdmin,
  weeklyDigestEnabled,
}: GroupInsightsPanelProps) {
  const { t } = useTranslation();
  const enabled = isAdmin && weeklyDigestEnabled;
  const { report, isLoading: isReportLoading } = useGroupWeeklyReport(groupId, enabled);
  const { flags, isLoading: isFlagsLoading } = useGroupHealthFlags(groupId, enabled);

  if (!isAdmin) {
    return null;
  }

  if (!weeklyDigestEnabled) {
    return (
      <div data-testid="group-insights-tier-locked">
        <Card variant="dark" className="!p-5">
          <div className="flex items-start gap-3">
            <Info className="w-4 h-4 text-on-surface-variant mt-0.5 flex-shrink-0" />
            <div>
              <h4 className="text-sm font-semibold text-on-surface">{t('groups.weeklyReport')}</h4>
              <p className="text-sm text-on-surface-variant mt-1">
                {t('groups.insightsPlanRequired')}
              </p>
            </div>
          </div>
        </Card>
      </div>
    );
  }

  if (isReportLoading || isFlagsLoading) {
    return (
      <div className="flex justify-center py-8">
        <div className="pierre-spinner" />
      </div>
    );
  }

  return (
    <div className="space-y-4" data-testid="group-insights-panel">
      {report && (
        <Card variant="dark" className="!p-5">
          <div className="flex items-center gap-2 mb-3">
            <CalendarRange className="w-4 h-4 text-primary" />
            <h4 className="text-sm font-semibold text-on-surface">{t('groups.thisWeek')}</h4>
          </div>
          <p className="text-sm text-on-surface-variant" data-testid="group-report-summary">
            {report.summary}
          </p>

          {report.highlights.length > 0 && (
            <div className="mt-4">
              <p className="text-xs font-medium text-on-surface-variant mb-2">{t('groups.highlights')}</p>
              <ul className="space-y-1.5">
                {report.highlights.map((highlight) => (
                  <li
                    key={highlight}
                    className="flex items-start gap-2 text-sm text-on-surface"
                    data-testid="group-report-highlight"
                  >
                    <CheckCircle2 className="w-3.5 h-3.5 text-activity mt-0.5 flex-shrink-0" />
                    <span>{highlight}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {report.concerns.length > 0 && (
            <div className="mt-4">
              <p className="text-xs font-medium text-on-surface-variant mb-2">{t('groups.concerns')}</p>
              <ul className="space-y-1.5">
                {report.concerns.map((concern) => (
                  <li
                    key={concern}
                    className="flex items-start gap-2 text-sm text-on-surface"
                    data-testid="group-report-concern"
                  >
                    <AlertTriangle className="w-3.5 h-3.5 text-warning mt-0.5 flex-shrink-0" />
                    <span>{concern}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {report.recommendations.length > 0 && (
            <div className="mt-4">
              <p className="text-xs font-medium text-on-surface-variant mb-2">{t('groups.recommendations')}</p>
              <ul className="space-y-1.5">
                {report.recommendations.map((recommendation) => (
                  <li
                    key={recommendation}
                    className="flex items-start gap-2 text-sm text-on-surface"
                    data-testid="group-report-recommendation"
                  >
                    <Lightbulb className="w-3.5 h-3.5 text-primary mt-0.5 flex-shrink-0" />
                    <span>{recommendation}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </Card>
      )}

      <Card variant="dark" className="!p-5">
        <div className="flex items-center gap-2 mb-3">
          <AlertTriangle className="w-4 h-4 text-warning" />
          <h4 className="text-sm font-semibold text-on-surface">
            {t('groups.healthFlagsCount', { n: flags.length })}
          </h4>
        </div>
        {flags.length === 0 ? (
          <p className="text-sm text-outline" data-testid="group-health-flags-empty">
            {t('groups.noneFlagged')}
          </p>
        ) : (
          <ul className="space-y-2">
            {flags.map((flag) => (
              <li
                key={`${flag.user_id}-${flag.flag_type}`}
                className="flex items-start justify-between gap-3"
                data-testid="group-health-flag-row"
              >
                <div className="min-w-0">
                  <p className="text-sm font-medium text-on-surface truncate">
                    {flag.display_name}
                  </p>
                  <p className="text-xs text-on-surface-variant mt-0.5">{flag.detail}</p>
                </div>
                <span
                  className={`px-2 py-0.5 rounded-full text-[11px] font-medium flex-shrink-0 ${SEVERITY_STYLES[flag.severity]}`}
                >
                  {t(FLAG_LABEL_KEYS[flag.flag_type])}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}
