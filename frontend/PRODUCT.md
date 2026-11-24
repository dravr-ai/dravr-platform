# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The web app is also a first-class mobile surface: it must work as a responsive, installable PWA-style app that feels native on a phone, alongside the separate native app in `frontend-mobile/`. Mobile web remains `web`; it does not take OS-specific design languages.

## Users

Primary: endurance athletes (cyclists, triathletes, runners) who use the app daily to talk to the AI agent, see Today, and follow training plans, often from a phone.

Second: human coaches who run plans, groups and replies for the athletes they coach. Admins use an admin console in the same app (user management, usage, keys). A coach is a person; the agent is the AI. The two are never conflated in the UI or copy.

## Product Purpose

Dravr is AI fitness coaching built on the athlete's real data from connected providers (Strava, Garmin, Whoop, Wahoo, Intervals.icu and others). The web app is where athletes converse with the agent, review training and recovery, and manage connections, and where coaches build plans and work with groups. Success is an athlete who trusts the answer enough to act on it.

## Positioning

Coaching claims are verified against the athlete's real data, and the AI agent is clearly separate from the human coach. A neighbouring product built on an AI agent that does not verify, or that blurs agent and coach, cannot truthfully make this claim.

## Operating Context

Athletes use it in short, frequent sessions around training, often on a phone; coaches use it at a desk or on a phone to edit plans and answer athletes. Data arrives from provider integrations and the app is multi-tenant. A messaging surface (chat apps) sits beside the app, so conversation continuity matters.

## Capabilities and Constraints

Surfaces in the codebase: home and Today, chat and conversations, activity, plans and coach plan editing, coach home and replies, groups, connections, memory, notifications, onboarding, settings, store.

Terminology: "agent" = the AI, "coach" = a person. ACWR and load ratios are descriptive magnitudes, never injury risk. Form is banded as a share of the athlete's own CTL, never an absolute TSB and never compared across athletes. Training science lives in a separate crate (dravr-cageux); the UI never re-derives it. Shared types and API methods come from `@pierre/shared-types` and `@pierre/api-client`.

Not yet decided: the product's long-term tone differences between the athlete and coach roles.

## Brand Commitments

Brand name Dravr. The Boreal design system (see DESIGN.md and BRAND.md) is the existing, binding visual identity; this record does not extend or change it. Voice is calm and premium, never hype (per DESIGN.md).

## Evidence on Hand

Real provider data and a small alpha cohort of testers. No public customer logos, testimonials or benchmarks exist in the repo; future work must not fabricate them.

## Product Principles

1. Verified over fluent: a claim the data does not support does not ship.
2. Agent and coach stay distinct, in copy, avatars and flow.
3. Phone first: every screen works one-handed on a small viewport before it scales up.
4. Describe, never alarm: metrics are magnitudes and context, not warnings; form is relative to the athlete's own fitness.
5. One conversation, many surfaces: the goal is that web, native and messaging continue the same conversation.

## Accessibility & Inclusion

Text and surface pairings follow the contrast ratios documented in DESIGN.md; both light and dark themes must hold them. Multi-language support is required (i18n package in the repo).
