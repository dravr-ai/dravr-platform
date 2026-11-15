// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What an athlete's group does with what they share, in plain sentences — the coach, the agent, the plan thread
// ABOUTME: Each sentence follows the group's real state: the athlete's coach sharing, the agent's reply mode, whether a coach is there

import type { CoachingGroup } from '@pierre/shared-types';
import { MENTION_PREFIX } from '@pierre/shared-constants';
import { useTranslation } from '@pierre/i18n';
import { ROW_DIVIDER } from './GroupInfoParts';

interface WhoSeesWhatProps {
  group: Pick<CoachingGroup, 'agent_title' | 'agent_handle' | 'respond_mode' | 'coach_user_id' | 'coach_display_name'>;
  /**
   * Whether the athlete shares their training with the group's coach, once
   * read; `null` while it loads, so the coach's sentence never says the
   * wrong thing first.
   */
  coachSharing: boolean | null;
}

/**
 * Visibility comes before the choices about it (the P2·B board): who reads
 * this room and what each of them sees, said once, as the group really is.
 * The coach's sentence follows the athlete's own switch, the agent's follows
 * how the group set it to reply, and the plan sentence appears only once a
 * coach is there to write a plan.
 */
export default function WhoSeesWhat({ group, coachSharing }: WhoSeesWhatProps) {
  const { t } = useTranslation();
  const agent = group.agent_title ?? t('app.aiAgent');
  const coach = group.coach_user_id ? (group.coach_display_name ?? t('humanCoach.attached')) : null;

  const agentLine =
    group.respond_mode === 'mentions'
      ? group.agent_handle
        ? t('groups.info.agentMentions', { agent, handle: `${MENTION_PREFIX}${group.agent_handle}` })
        : t('groups.info.agentMentionsNoHandle', { agent })
      : t('groups.info.agentEvery', { agent });

  return (
    <ul className={`space-y-1.5 px-4 py-3 ${ROW_DIVIDER}`} data-testid="group-info-who-sees">
      {coach !== null && coachSharing !== null && (
        <li className="text-pretty text-sm text-on-surface-variant" data-testid="who-sees-coach">
          {coachSharing ? t('humanCoach.whoSees.sharing', { coach }) : t('humanCoach.whoSees.notSharing', { coach })}
        </li>
      )}
      <li className="text-pretty text-sm text-on-surface-variant" data-testid="who-sees-agent">
        {agentLine}
      </li>
      {coach !== null && (
        <li className="text-pretty text-sm text-on-surface-variant" data-testid="who-sees-plan">
          {t('humanCoach.whoSees.planThread', { coach, agent })}
        </li>
      )}
    </ul>
  );
}
