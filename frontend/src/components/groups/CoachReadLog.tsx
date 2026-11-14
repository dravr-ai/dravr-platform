// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Group info's record of the human coach's reads of the member's data — who, which kind, through what, when
// ABOUTME: Shown beside the member's coach sharing; the server lists the member's own reads only, newest first, kept a year

import type { CoachRead } from '@pierre/shared-types';
import { formatDateTime } from '@pierre/chat-utils';
import { COACH_READ_LOG_KEYS, coachReadClassKey, coachReadSourceKey } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { useCoachReads } from '../../hooks/useGroups';

interface CoachReadLogProps {
  groupId: string;
}

/**
 * One read: who read, which kind of data, through what, and when. A read made
 * outside a tool call — the plan view, `/plan`, the coach's roster, the
 * coach's thread — reads in plain words; a tool is named as it is.
 */
function ReadRow({ read }: { read: CoachRead }) {
  const { t, language } = useTranslation();
  const sourceKey = coachReadSourceKey(read.tool);
  return (
    <li className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5 py-1.5" data-testid="coach-read-row">
      <span className="text-sm text-on-surface">
        <span className="font-medium" data-testid="coach-read-coach">
          {read.coach_name ?? t(COACH_READ_LOG_KEYS.formerCoach)}
        </span>
        {' · '}
        <span data-testid="coach-read-class">{t(coachReadClassKey(read.data_class))}</span>
        {' · '}
        {sourceKey ? (
          <span className="text-on-surface-variant" data-testid="coach-read-tool">
            {t(sourceKey)}
          </span>
        ) : (
          <code className="text-xs text-on-surface-variant" data-testid="coach-read-tool">
            {read.tool}
          </code>
        )}
      </span>
      <time className="text-xs text-outline" dateTime={read.read_at} data-testid="coach-read-at">
        {formatDateTime(read.read_at, language)}
      </time>
    </li>
  );
}

/**
 * The member's own record of the group's human coach reading their data
 * through their agent (carnet#1445): each read the coach's agent made of
 * their training or wellness, newest first, kept for the retention the
 * server names. Only the member's own reads in this group reach it.
 */
export default function CoachReadLog({ groupId }: CoachReadLogProps) {
  const { t } = useTranslation();
  const { coachReads, isLoading, isError } = useCoachReads(groupId, true);

  return (
    <div className="space-y-2" data-testid="coach-read-log">
      <p className="text-sm font-medium text-on-surface">{t(COACH_READ_LOG_KEYS.title)}</p>
      <p className="text-xs text-on-surface-variant">{coachReads && t(COACH_READ_LOG_KEYS.hint, { months: coachReads.retention_months })}</p>
      {isError ? (
        <p className="text-xs text-error" role="alert" data-testid="coach-read-log-error">
          {t(COACH_READ_LOG_KEYS.loadFailed)}
        </p>
      ) : isLoading || !coachReads ? null : coachReads.reads.length === 0 ? (
        <p className="text-xs text-on-surface-variant" data-testid="coach-read-log-empty">
          {t(COACH_READ_LOG_KEYS.empty, { months: coachReads.retention_months })}
        </p>
      ) : (
        <ul className="divide-y divide-outline-variant/40">
          {coachReads.reads.map((read) => (
            <ReadRow key={`${read.read_at}-${read.tool}-${read.coach_name ?? ''}`} read={read} />
          ))}
        </ul>
      )}
    </div>
  );
}
