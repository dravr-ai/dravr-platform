// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's open draft drawn in their plan thread when no card in it shows that draft — the badge, Publish and Discard
// ABOUTME: A draft reaches a thread on the reply that saved it; one saved anywhere else is still read and shown here

import React from 'react';
import { View } from 'react-native';
import type { OpenPlanDraft } from '@pierre/ui-logic';
import WorkoutPlanCard, { type SessionEditorTarget } from './WorkoutPlanCard';
import PlanDraftControls from './PlanDraftControls';

interface PlanThreadOpenDraftProps {
  /** The coach's plan thread on the athlete. */
  conversationId: string;
  /** The draft the subject-plan read holds open. */
  open: OpenPlanDraft;
  /** Opens the coach's day editor on one of the draft's entries, or on a date to add one (carnet#1515, #1714, #1716). */
  onBuildDay?: (target: SessionEditorTarget) => void;
}

/** The draft "Review the draft" sent the coach here to read, with its controls. */
export default function PlanThreadOpenDraft({ conversationId, open, onBuildDay }: PlanThreadOpenDraftProps) {
  return (
    <View className="mt-4 w-full" testID="chat-plan-thread-open-draft">
      <WorkoutPlanCard plan={open.plan} onBuildDay={onBuildDay}>
        <PlanDraftControls conversationId={conversationId} subject={open.subject} draft={open.draft} />
      </WorkoutPlanCard>
    </View>
  );
}
