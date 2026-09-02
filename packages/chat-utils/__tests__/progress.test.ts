// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the shared turn-progress → catalogue key mapping
// ABOUTME: Pins the key and params both chat UIs translate, and the events they must hide

import { describe, it, expect } from 'vitest';
import fs from 'fs';
import path from 'path';
import type { TurnProgress } from '@pierre/shared-types';
import { statusForProgress, THINKING_PLACEHOLDER_KEY } from '../src/progress';

function stage(title: string, status: string): TurnProgress {
  return { kind: 'stage', id: title, title, status };
}

function tool(title: string, status: string): TurnProgress {
  return { kind: 'tool', id: 'call_1', title, status };
}

describe('statusForProgress', () => {
  it('names the two pipeline stages by their catalogue keys', () => {
    expect(statusForProgress(stage('prompt_assembly', 'started'))).toEqual({
      key: 'chat.status.readingQuestion',
    });
    expect(statusForProgress(stage('dispatch', 'started'))).toEqual({
      key: 'chat.status.generatingResponse',
    });
  });

  it('falls back to the stage name for a stage it has no phrasing for', () => {
    expect(statusForProgress(stage('memory_extraction', 'started'))).toEqual({
      key: 'chat.status.stage',
      params: { stage: 'memory_extraction' },
    });
  });

  it('hides a finished stage — the next event replaces it anyway', () => {
    expect(statusForProgress(stage('dispatch', 'finished'))).toBeNull();
    expect(statusForProgress(stage('prompt_assembly', 'finished'))).toBeNull();
  });

  it('names the running tool, and clears back to the placeholder when it completes', () => {
    expect(statusForProgress(tool('get_activities', 'InProgress'))).toEqual({
      key: 'chat.status.callingTool',
      params: { tool: 'get_activities' },
    });
    expect(statusForProgress(tool('get_activities', 'Pending'))).toEqual({
      key: 'chat.status.callingTool',
      params: { tool: 'get_activities' },
    });
    expect(statusForProgress(tool('get_activities', 'Completed'))).toEqual({
      key: THINKING_PLACEHOLDER_KEY,
    });
  });

  it('describes an unnamed tool generically rather than rendering an empty line', () => {
    expect(statusForProgress(tool('', 'InProgress'))).toEqual({ key: 'chat.status.runningTool' });
  });

  it('emits only keys the catalogue carries, in every locale', () => {
    // A key with no entry renders as the dotted key itself on the screen —
    // worse than the English it replaced. Both halves are pinned here because
    // this module cannot import the catalogue at runtime.
    const emitted = [
      statusForProgress(stage('prompt_assembly', 'started')),
      statusForProgress(stage('dispatch', 'started')),
      statusForProgress(stage('memory_extraction', 'started')),
      statusForProgress(tool('get_activities', 'InProgress')),
      statusForProgress(tool('', 'InProgress')),
      statusForProgress(tool('get_activities', 'Completed')),
    ].map(status => status?.key ?? '');

    for (const language of ['fr', 'en', 'es', 'de', 'pt']) {
      const bundle = JSON.parse(
        fs.readFileSync(
          path.join(__dirname, `../../i18n/src/locales/${language}/translation.json`),
          'utf-8',
        ),
      ) as Record<string, unknown>;
      for (const key of emitted) {
        const value = key
          .split('.')
          .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], bundle);
        expect(typeof value, `${language} is missing ${key}`).toBe('string');
      }
    }
  });
});
