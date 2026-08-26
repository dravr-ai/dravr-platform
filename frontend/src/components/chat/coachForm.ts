// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Conversions between the coach editor's form state and the coaches API
// ABOUTME: One source of truth for create/update payloads, shared by every mount of CoachFormModal

import type { Coach, CreateCoachRequest, UpdateCoachRequest } from '@pierre/shared-types';
import type { CoachFormData } from './types';

/** Hydrate the coach editor's form state from a stored coach. */
export function coachToFormData(coach: Coach): CoachFormData {
  const dr = coach.data_requirements;
  return {
    title: coach.title,
    description: coach.description || '',
    system_prompt: coach.system_prompt,
    category: coach.category,
    startup_query: coach.startup_query || '',
    prefetch_enabled: !!dr?.activities,
    activity_count: dr?.activities?.count ?? 20,
    sport_types: dr?.activities?.sport_types ?? [],
    time_frame: dr?.activities?.time_frame ?? '12w',
    detail_mode: (dr?.activities?.mode as 'summary' | 'detailed') ?? 'summary',
    athlete_profile: dr?.athlete_profile ?? false,
    purpose: coach.purpose || '',
    when_to_use: coach.when_to_use || '',
    instructions: coach.instructions || '',
    example_inputs: coach.example_inputs || '',
    example_outputs: coach.example_outputs || '',
    success_criteria: coach.success_criteria || '',
    // A coach with no stored override hydrates an empty box, so re-saving it
    // from this form leaves it inheriting rather than pinning today's default.
    max_tool_iterations: coach.max_tool_iterations,
  };
}

/** Convert UI form data to API request, building data_requirements from structured fields */
export function formDataToCreateRequest(data: CoachFormData): CreateCoachRequest {
  const request: CreateCoachRequest = {
    title: data.title,
    description: data.description || undefined,
    system_prompt: data.system_prompt,
    category: data.category,
  };

  // Send the budget only when the user typed one. A new coach has no stored pin
  // to clear, so both untouched (`undefined`) and emptied (`null`) omit the key
  // and leave the coach inheriting the tenant-wide
  // `tool_execution.max_iterations` an admin can raise.
  if (typeof data.max_tool_iterations === 'number') {
    request.max_tool_iterations = data.max_tool_iterations;
  }

  if (data.startup_query.trim()) {
    request.startup_query = data.startup_query.trim();
  }

  if (data.prefetch_enabled) {
    request.data_requirements = {
      activities: {
        count: data.activity_count,
        sport_types: data.sport_types,
        time_frame: data.time_frame,
        mode: data.detail_mode,
        format: 'toon',
        analysis_type: 'general_overview',
      },
      athlete_profile: data.athlete_profile,
    };
  }

  // Pass structured sections when provided
  if (data.purpose.trim()) request.purpose = data.purpose.trim();
  if (data.when_to_use.trim()) request.when_to_use = data.when_to_use.trim();
  if (data.instructions.trim()) request.instructions = data.instructions.trim();
  if (data.example_inputs.trim()) request.example_inputs = data.example_inputs.trim();
  if (data.example_outputs.trim()) request.example_outputs = data.example_outputs.trim();
  if (data.success_criteria.trim()) request.success_criteria = data.success_criteria.trim();

  return request;
}

/** Convert UI form data to API update request */
export function formDataToUpdateRequest(data: CoachFormData): UpdateCoachRequest {
  const request: UpdateCoachRequest = {
    title: data.title,
    description: data.description || undefined,
    system_prompt: data.system_prompt,
    category: data.category,
    startup_query: data.startup_query.trim() || undefined,
  };

  // Three-way, and the difference matters: an untouched field (`undefined`) is
  // left out so the server preserves whatever is stored, while an emptied box
  // (`null`) is sent as an explicit null so a coach that already carries a pin
  // can be reset to inheriting the tenant-wide limit.
  if (data.max_tool_iterations !== undefined) {
    request.max_tool_iterations = data.max_tool_iterations;
  }

  if (data.prefetch_enabled) {
    request.data_requirements = {
      activities: {
        count: data.activity_count,
        sport_types: data.sport_types,
        time_frame: data.time_frame,
        mode: data.detail_mode,
        format: 'toon',
        analysis_type: 'general_overview',
      },
      athlete_profile: data.athlete_profile,
    };
  }

  // Pass structured sections when provided
  if (data.purpose.trim()) request.purpose = data.purpose.trim();
  if (data.when_to_use.trim()) request.when_to_use = data.when_to_use.trim();
  if (data.instructions.trim()) request.instructions = data.instructions.trim();
  if (data.example_inputs.trim()) request.example_inputs = data.example_inputs.trim();
  if (data.example_outputs.trim()) request.example_outputs = data.example_outputs.trim();
  if (data.success_criteria.trim()) request.success_criteria = data.success_criteria.trim();

  return request;
}
