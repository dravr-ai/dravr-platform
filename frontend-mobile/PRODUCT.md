# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

One product on iOS and Android that uses native navigation, gestures, Dynamic Type and system UI within the one shared Boreal look. The web app in `frontend/` is separately responsive and PWA-capable; this record covers only the native app.

## Users

Primary: endurance athletes (cyclists, triathletes, runners) who use the app daily to talk to the AI agent, see Today, and follow training plans, often around training sessions.

Second: human coaches who run plans, groups and replies for the athletes they coach. A coach is a person; the agent is the AI. The two are never conflated in the UI or copy.

## Product Purpose

Dravr is AI fitness coaching built on the athlete's real data from connected providers (Strava, Garmin, Whoop, Wahoo, Intervals.icu and others). The native app is the athlete's pocket surface for conversing with the agent, reviewing training and recovery, and following plans; coaches use it to work with athletes and groups. Success is an athlete who trusts the answer enough to act on it.

## Positioning

Coaching claims are verified against the athlete's real data, and the AI agent is clearly separate from the human coach. A neighbouring product built on an AI agent that does not verify, or that blurs agent and coach, cannot truthfully make this claim.

## Operating Context

Short, frequent sessions, often one-handed, on a phone around training. Push notifications bring athletes back. Data arrives from provider integrations and the app is multi-tenant. A messaging surface (chat apps) sits beside the app, so conversation continuity matters.

## Capabilities and Constraints

Screens in the codebase: home, chat and conversations, activity, coach home and replies, coaches, groups, connections, memory, notifications, onboarding, settings, store, import.

Terminology: "agent" = the AI, "coach" = a person. ACWR and load ratios are descriptive magnitudes, never injury risk. Form is banded as a share of the athlete's own CTL, never an absolute TSB and never compared across athletes. Training science lives in a separate crate (dravr-cageux); the UI never re-derives it. Shared types and API methods come from `@pierre/shared-types` and `@pierre/api-client`. Built with React Native / Expo and NativeWind.

## Brand Commitments

Brand name Dravr. The Boreal design system (shared with web, documented in `frontend/DESIGN.md`) is the existing, binding visual identity; this record does not extend or change it. Voice is calm and premium, never hype.

## Evidence on Hand

Real provider data and a small alpha cohort of testers. No public customer logos, testimonials or benchmarks exist in the repo; future work must not fabricate them.

## Product Principles

1. Verified over fluent: a claim the data does not support does not ship.
2. Agent and coach stay distinct, in copy, avatars and flow.
3. Native first: native navigation, gestures, Dynamic Type and system UI, within the one Boreal look on both OSes.
4. Describe, never alarm: metrics are magnitudes and context, not warnings; form is relative to the athlete's own fitness.
5. One conversation, many surfaces: the goal is that native, web and messaging continue the same conversation.

## Accessibility & Inclusion

Text and surface pairings follow the contrast ratios documented in `frontend/DESIGN.md`; both light and dark themes must hold them. Respect OS accessibility settings (dynamic type, reduced motion, screen readers). Multi-language support is required (i18n package in the repo).
