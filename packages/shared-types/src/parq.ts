// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The PAR-Q+ wire shapes — one question as `GET /api/me/parq` serves it, one answer as its `POST` takes it
// ABOUTME: Shared by both clients' health-check forms, so the onboarding screen and Home's sheet read one type (carnet#1530)

/** One PAR-Q+ question as the server serves it, verbatim in the athlete's language. */
export interface ParqQuestion {
  id: string;
  text: string;
}

/** The athlete's answer to one PAR-Q+ question: a "yes" raises an agent-visible flag. */
export interface ParqAnswer {
  id: string;
  yes: boolean;
}
