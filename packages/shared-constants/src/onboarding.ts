// ABOUTME: Declarative onboarding step registry — canonical order + per-step applicability/completion predicates
// ABOUTME: Single source of truth for which onboarding step is current; consumed by BOTH web and mobile

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/** Identifiers for the onboarding steps, in canonical journey order. */
export type OnboardingStepId =
  | 'profile_type'
  | 'connect_provider'
  | 'coach_proposal'
  | 'messaging_channel'
  | 'messaging_configure';

/**
 * The signals the flow derives "which step is current" from:
 *   - `onboardingActive` — authenticated, active, non-admin (admins are exempt
 *     from the whole flow).
 *   - `needsProviderConnection` — server truth; `undefined` while the query is
 *     in flight or disabled (e.g. admins).
 *   - `skippedProvider` — session-only "continue without connecting".
 *   - `justOnboarded` — a `needs_provider_connection` true→false transition
 *     observed *this session*, so coach-proposal never intercepts a returning
 *     already-onboarded user simply logging in.
 *   - `profileTypeChosen` / `coachProposalDone` — per-user local flags.
 */
export interface OnboardingContext {
  onboardingActive: boolean;
  needsProviderConnection: boolean | undefined;
  skippedProvider: boolean;
  justOnboarded: boolean;
  profileTypeChosen: boolean;
  coachProposalDone: boolean;
  /** How many messaging channels the tenant has configured (0 ⇒ skip messaging). */
  messagingAvailableCount: number;
  /** A messaging channel is chosen — picked, or auto-selected when only one exists. */
  messagingChannelChosen: boolean;
  /** The messaging-channel picker step is satisfied (picked or skipped). */
  messagingChannelDone: boolean;
  /** The messaging-configure step is satisfied (channel linked or skipped). */
  messagingConfigureDone: boolean;
}

/** A single step in the onboarding journey. */
export interface OnboardingStepDef {
  id: OnboardingStepId;
  /** Short label for the progress indicator. */
  label: string;
  /**
   * Whether this step belongs to *this* user's journey right now. Phase-gated:
   * profile-type and connect-provider belong to the pre-connect phase
   * (`needs === true`); coach-proposal to the just-connected phase; the messaging
   * steps to the post-connect phase (`needs === false`).
   */
  isApplicable: (ctx: OnboardingContext) => boolean;
  /**
   * Whether this step has been satisfied. Drives both the decision engine
   * (step past a complete step) and the progress indicator's "done" state, so
   * it is defined phase-agnostically (see each step).
   */
  isComplete: (ctx: OnboardingContext) => boolean;
}

/**
 * Canonical onboarding pipeline, in order. The current step is the first
 * applicable step that is not yet complete (see `currentOnboardingStep`); when
 * none remain, onboarding is done and the dashboard/chat renders.
 */
export const ONBOARDING_STEPS: OnboardingStepDef[] = [
  {
    id: 'profile_type',
    label: 'About you',
    isApplicable: (c) =>
      c.onboardingActive && c.needsProviderConnection === true && !c.skippedProvider,
    isComplete: (c) => c.profileTypeChosen,
  },
  {
    id: 'connect_provider',
    label: 'Connect',
    isApplicable: (c) => c.needsProviderConnection === true && !c.skippedProvider,
    // "Complete" once a provider is connected. Only ever evaluated for the
    // progress indicator: while the step is applicable `needs === true`, so this
    // reads false; once a provider connects, `isApplicable` drops the step.
    isComplete: (c) => c.needsProviderConnection === false,
  },
  {
    id: 'coach_proposal',
    label: 'Coach',
    isApplicable: (c) =>
      c.onboardingActive && c.justOnboarded && c.needsProviderConnection === false,
    isComplete: (c) => c.coachProposalDone,
  },
  {
    // Pick which messaging app to connect. Shown post-connect when the tenant has
    // MORE than one channel configured — a single channel is auto-selected, so we
    // never ask the user to "pick" from a list of one (isComplete short-circuits).
    // Unlike coach-proposal this is NOT gated on the in-session `justOnboarded`
    // transition: an already-connected user who hasn't set up messaging is
    // prompted once, and the done/skip flags stop re-prompts.
    id: 'messaging_channel',
    label: 'Chat app',
    isApplicable: (c) =>
      c.onboardingActive &&
      c.needsProviderConnection === false &&
      c.messagingAvailableCount > 1,
    isComplete: (c) => c.messagingChannelDone || c.messagingAvailableCount <= 1,
  },
  {
    // Configure/link the chosen messaging app (QR + deep link, or OAuth redirect).
    // Applicable once a channel is chosen; auto-satisfied when no channel exists.
    id: 'messaging_configure',
    label: 'Link',
    isApplicable: (c) =>
      c.onboardingActive &&
      c.needsProviderConnection === false &&
      c.messagingAvailableCount >= 1 &&
      c.messagingChannelChosen,
    isComplete: (c) => c.messagingConfigureDone || c.messagingAvailableCount === 0,
  },
];

/**
 * The current onboarding step: the first applicable step not yet complete, or
 * `null` when onboarding is finished (→ render the dashboard/chat).
 */
export function currentOnboardingStep(ctx: OnboardingContext): OnboardingStepDef | null {
  return ONBOARDING_STEPS.filter((s) => s.isApplicable(ctx)).find((s) => !s.isComplete(ctx)) ?? null;
}

/** A persisted onboarding step record as returned by the server (structural). */
export interface PersistedOnboardingStep {
  step_id: string;
  status: string;
}

/**
 * Whether the server's durable record marks `stepId` as done (`complete` or
 * `skipped`). This is what makes onboarding survive device changes: a step
 * finished on one device is stepped past on another even with no local flag.
 */
export function isServerStepComplete(
  steps: PersistedOnboardingStep[] | undefined,
  stepId: OnboardingStepId,
): boolean {
  return (steps ?? []).some(
    (s) => s.step_id === stepId && (s.status === 'complete' || s.status === 'skipped'),
  );
}

/** Display status for a step in the progress indicator. */
export type OnboardingStepStatus = 'done' | 'current' | 'upcoming';

/** A step plus its progress-indicator display status. */
export interface OnboardingProgressItem {
  id: OnboardingStepId;
  label: string;
  status: OnboardingStepStatus;
}

/**
 * The full canonical pipeline with each step's display status — a stable
 * sequence (unlike the phase-gated applicable set) so the progress indicator
 * doesn't reflow between steps.
 *
 * Status is *position-based*: the current step is `current`, everything before
 * it is `done`, everything after is `upcoming`. This deliberately does NOT reuse
 * each step's `isComplete` — an auto-skipped step (e.g. the messaging picker when
 * only one channel exists) would otherwise read `done` while the user is still on
 * step one. Position keeps the bar honest: nothing reads done until it's behind us.
 */
export function onboardingProgress(ctx: OnboardingContext): OnboardingProgressItem[] {
  const currentId = currentOnboardingStep(ctx)?.id;
  const currentIndex = currentId
    ? ONBOARDING_STEPS.findIndex((s) => s.id === currentId)
    : ONBOARDING_STEPS.length; // onboarding finished — everything is behind us
  return ONBOARDING_STEPS.map((s, i) => ({
    id: s.id,
    label: s.label,
    status: i === currentIndex ? 'current' : i < currentIndex ? 'done' : 'upcoming',
  }));
}
