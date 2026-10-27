// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Holds the step table to the tools a chat turn can call — every one names its step, every step names a tool
// ABOUTME: A tool added to the server without a step reads as "looking things up"; this fails before an athlete sees that

import { describe, expect, it } from 'vitest';
import { TOOL_NAMES } from '@pierre/mcp-types';
import CHAT_CALLABLE_TOOLS from '@pierre/mcp-types/chat-callable-tools.json';
import { TURN_STEP_KEYS, toolStepLabel } from '../src/turn-trail';

// The list a chat turn is offered, frozen from the server's
// `chat_callable_schemas()`; `chat_callable_tools_fixture_test` in
// crates/pierre-server holds it to the registry in both directions.
const TURN_TOOLS: readonly string[] = CHAT_CALLABLE_TOOLS;

/** The keys a turn's own phases use rather than a tool: never a tool's family. */
const PHASE_KEYS: ReadonlySet<string> = new Set([
  TURN_STEP_KEYS.thinking,
  TURN_STEP_KEYS.readingQuestion,
  TURN_STEP_KEYS.analysing,
  TURN_STEP_KEYS.writing,
  TURN_STEP_KEYS.lookingUp,
]);

describe('step table against the tools a turn can call', () => {
  it('names a family for every tool a turn can call, never a phase', () => {
    expect(TURN_TOOLS.length).toBeGreaterThan(0);
    const unnamed = TURN_TOOLS.filter(name => PHASE_KEYS.has(toolStepLabel(name).key));
    expect(unnamed).toEqual([]);
  });

  it('leaves no step family without a callable tool to name', () => {
    const used = new Set(TURN_TOOLS.map(name => toolStepLabel(name).key));
    const unused = Object.values(TURN_STEP_KEYS).filter(key => !PHASE_KEYS.has(key) && !used.has(key));
    expect(unused).toEqual([]);
  });

  it('maps no tool a turn cannot call, so the table holds only steps an athlete can see', () => {
    const callable: ReadonlySet<string> = new Set(TURN_TOOLS);
    const mapped = TOOL_NAMES.filter(
      name => !callable.has(name) && toolStepLabel(name).key !== TURN_STEP_KEYS.lookingUp,
    );
    expect(mapped).toEqual([]);
  });

  it('reads a runner-qualified title as the tool it names', () => {
    expect(toolStepLabel('pierre__push_training_plan')).toEqual({ key: TURN_STEP_KEYS.updateCalendar });
    expect(toolStepLabel('mcp/get_recipe_constraints')).toEqual({ key: TURN_STEP_KEYS.recipes });
    expect(toolStepLabel('pierre__install_agent_from_store')).toEqual({ key: TURN_STEP_KEYS.addAgent });
    expect(toolStepLabel('mcp/browse_agent_store')).toEqual({ key: TURN_STEP_KEYS.browseAgents });  });
});
