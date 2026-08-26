// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Unit tests for the shared turn-progress → status text mapping
// ABOUTME: Pins the exact strings both chat UIs render, and the events they must hide

import { describe, it, expect } from 'vitest';
import type { TurnProgress } from '@pierre/shared-types';
import { statusTextForProgress, THINKING_PLACEHOLDER } from '../src/progress';

function stage(title: string, status: string): TurnProgress {
  return { kind: 'stage', id: title, title, status };
}

function tool(title: string, status: string): TurnProgress {
  return { kind: 'tool', id: 'call_1', title, status };
}

describe('statusTextForProgress', () => {
  it('names the two pipeline stages in the athlete’s own words', () => {
    expect(statusTextForProgress(stage('prompt_assembly', 'started'))).toBe(
      'reading your question…',
    );
    expect(statusTextForProgress(stage('dispatch', 'started'))).toBe('generating response…');
  });

  it('falls back to the stage name for a stage it has no phrasing for', () => {
    expect(statusTextForProgress(stage('memory_extraction', 'started'))).toBe(
      'memory_extraction…',
    );
  });

  it('hides a finished stage — the next event replaces it anyway', () => {
    expect(statusTextForProgress(stage('dispatch', 'finished'))).toBeNull();
    expect(statusTextForProgress(stage('prompt_assembly', 'finished'))).toBeNull();
  });

  it('names the running tool, and clears back to the placeholder when it completes', () => {
    expect(statusTextForProgress(tool('get_activities', 'InProgress'))).toBe(
      'calling get_activities…',
    );
    expect(statusTextForProgress(tool('get_activities', 'Pending'))).toBe(
      'calling get_activities…',
    );
    expect(statusTextForProgress(tool('get_activities', 'Completed'))).toBe(
      THINKING_PLACEHOLDER,
    );
  });

  it('describes an unnamed tool generically rather than rendering an empty line', () => {
    expect(statusTextForProgress(tool('', 'InProgress'))).toBe('running a tool…');
  });
});
