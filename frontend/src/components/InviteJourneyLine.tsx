// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One thin line naming the invite an account is on its way to joining, above the onboarding steps (carnet#1649)
// ABOUTME: Shown while an invite is carried or held and not joined, so the person never loses sight of why they are here

import { useTranslation } from '@pierre/i18n';
import { inviteJourneyLine } from '@pierre/ui-logic';
import { useInviteJourney } from '../hooks/useInviteJourney';

export default function InviteJourneyLine() {
  const { t } = useTranslation();
  const journey = useInviteJourney();
  if (journey === null) return null;
  const words = inviteJourneyLine({ inviter_name: journey.inviterName, group_name: journey.groupName });
  return (
    <p
      className="border-b ghost-border bg-surface-container-low px-4 py-2 text-center text-xs text-on-surface-variant"
      data-testid="invite-journey-line"
    >
      {t(words.key, words.values)}
    </p>
  );
}
