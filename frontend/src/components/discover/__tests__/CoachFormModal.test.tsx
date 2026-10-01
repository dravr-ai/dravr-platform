// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the CoachFormModal tool-budget input and its delete affordance
// ABOUTME: Verifies the stored budget renders, edits propagate, bounds hold, clearing is explicit, prompt size reads localised

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
} from '@pierre/shared-constants';
import { i18n } from '@pierre/i18n';
import CoachFormModal from '../CoachFormModal';
import { DEFAULT_COACH_FORM_DATA, type AgentFormData } from '../coachForm';

function makeFormData(overrides: Partial<AgentFormData> = {}): AgentFormData {
  return {
    ...DEFAULT_COACH_FORM_DATA,
    title: 'Marathon Coach',
    system_prompt: 'You are an expert marathon coach.',
    ...overrides,
  };
}

function renderModal(
  formData: AgentFormData,
  onFormDataChange = vi.fn(),
  options: { onDelete?: () => void } = {},
) {
  render(
    <CoachFormModal
      isOpen
      formData={formData}
      onFormDataChange={onFormDataChange}
      onSubmit={vi.fn()}
      onClose={vi.fn()}
      isSubmitting={false}
      submitError={false}
      onDelete={options.onDelete}
    />,
  );
  return {
    onFormDataChange,
    input: screen.getByLabelText('Max tool iterations per turn') as HTMLInputElement,
  };
}

describe('CoachFormModal tool budget', () => {
  it('renders the agent’s stored budget', () => {
    const { input } = renderModal(makeFormData({ max_tool_iterations: 27 }));

    expect(input.value).toBe('27');
    expect(input.min).toBe(String(MIN_MAX_TOOL_ITERATIONS));
    expect(input.max).toBe(String(MAX_MAX_TOOL_ITERATIONS));
  });

  it('leaves an untouched budget empty so the agent inherits the workspace limit', () => {
    const { input } = renderModal(makeFormData());

    expect(DEFAULT_COACH_FORM_DATA.max_tool_iterations).toBeUndefined();
    expect(input.value).toBe('');
  });

  it('still communicates the effective default through the placeholder', () => {
    const { input } = renderModal(makeFormData());

    expect(input.placeholder).toBe(String(DEFAULT_MAX_TOOL_ITERATIONS));
  });

  it('submits an edited budget through onFormDataChange', () => {
    const { input, onFormDataChange } = renderModal(makeFormData({ max_tool_iterations: 10 }));

    fireEvent.change(input, { target: { value: '18' } });

    expect(onFormDataChange).toHaveBeenCalledTimes(1);
    const next = onFormDataChange.mock.calls[0][0] as AgentFormData;
    expect(next.max_tool_iterations).toBe(18);
    expect(next.title).toBe('Marathon Coach');
  });

  it('clamps a typed value above the ceiling down to the ceiling', () => {
    const { input, onFormDataChange } = renderModal(makeFormData({ max_tool_iterations: 10 }));

    fireEvent.change(input, { target: { value: '9000' } });

    const next = onFormDataChange.mock.calls[0][0] as AgentFormData;
    expect(next.max_tool_iterations).toBe(MAX_MAX_TOOL_ITERATIONS);
  });

  it('emptying the box on a pinned agent asks to clear, not to leave untouched', () => {
    const { input, onFormDataChange } = renderModal(makeFormData({ max_tool_iterations: 42 }));

    fireEvent.change(input, { target: { value: '' } });

    const next = onFormDataChange.mock.calls[0][0] as AgentFormData;
    // `null`, not `undefined`: undefined is the untouched state the request
    // omits, which would preserve the 42 the user just deleted.
    expect(next.max_tool_iterations).toBeNull();
  });

  it('renders an empty box for an agent whose pin was cleared', () => {
    const { input } = renderModal(makeFormData({ max_tool_iterations: null }));

    expect(input.value).toBe('');
  });
});

describe('CoachFormModal as the edit sheet', () => {
  it('is an edit form: no create-mode copy anywhere', () => {
    renderModal(makeFormData());

    expect(screen.getByRole('heading', { name: 'Edit Agent' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save Changes' })).toBeInTheDocument();
    expect(screen.queryByText('Create Custom Agent')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Create Agent' })).not.toBeInTheDocument();
  });

  it('offers "Delete this agent" only when the mount owns deletion', () => {
    const onDelete = vi.fn();
    renderModal(makeFormData(), vi.fn(), { onDelete });

    fireEvent.click(screen.getByRole('button', { name: 'Delete this agent' }));

    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it('renders no delete affordance without an onDelete handler', () => {
    renderModal(makeFormData());

    expect(screen.queryByRole('button', { name: 'Delete this agent' })).not.toBeInTheDocument();
  });
});

describe('CoachFormModal prompt size', () => {
  // 20,000 characters is 5,000 tokens: a figure large enough to need grouping
  // and a share (3.9%) with a decimal to write.
  const longPrompt = 'x'.repeat(20_000);

  it('reads the estimate in English notation', () => {
    renderModal(makeFormData({ system_prompt: longPrompt }));

    expect(screen.getByText('~5,000 tokens (3.9% of context)')).toBeInTheDocument();
  });

  it('reads the estimate as one French sentence in French notation', async () => {
    await i18n.changeLanguage('fr');
    try {
      // Rendered directly: renderModal finds the budget input by its English label.
      render(
        <CoachFormModal
          isOpen
          formData={makeFormData({ system_prompt: longPrompt })}
          onFormDataChange={vi.fn()}
          onSubmit={vi.fn()}
          onClose={vi.fn()}
          isSubmitting={false}
          submitError={false}
        />,
      );

      // Intl groups French thousands with a narrow no-break space (U+202F),
      // which the text matcher would collapse to a plain one.
      expect(screen.getByText(/^~5\s000 jetons/).textContent).toBe(
        '~5\u202f000 jetons (3,9 % du contexte)',
      );
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
