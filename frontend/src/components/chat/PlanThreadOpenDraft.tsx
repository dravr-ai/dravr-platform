// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's open draft drawn in their plan thread when no card in it shows that draft — the badge, Publish and Discard
// ABOUTME: A draft reaches a thread on the reply that saved it; one saved anywhere else is still read and shown here

import type { OpenPlanDraft } from '@pierre/ui-logic';
import WorkoutPlanCard from './WorkoutPlanCard';
import PlanDraftControls from './PlanDraftControls';

interface PlanThreadOpenDraftProps {
  /** The coach's plan thread on the athlete. */
  conversationId: string;
  /** The draft the subject-plan read holds open. */
  open: OpenPlanDraft;
}

/** The draft "Review the draft" sent the coach here to read, with its controls. */
export default function PlanThreadOpenDraft({ conversationId, open }: PlanThreadOpenDraftProps) {
  return (
    <div className="mt-2 w-full" data-testid="chat-plan-thread-open-draft">
      <WorkoutPlanCard plan={open.plan}>
        <PlanDraftControls conversationId={conversationId} subject={open.subject} draft={open.draft} />
      </WorkoutPlanCard>
    </div>
  );
}
