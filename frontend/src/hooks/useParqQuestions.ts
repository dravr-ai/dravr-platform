// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The seven PAR-Q+ questions in the athlete's language, read once per session
// ABOUTME: Shared by onboarding's Santé screen and Home's « Bilan de santé » sheet (carnet#1530)

import { useQuery } from '@tanstack/react-query';
import { userApi } from '../services/api';

/** `GET /api/me/parq`: the set never varies per athlete, so it is read once. */
export function useParqQuestions() {
  return useQuery({
    queryKey: ['parq-questions'],
    queryFn: () => userApi.getParqQuestions(),
    staleTime: Infinity,
  });
}
