// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the CoachFormModal tool-budget input and its delete affordance
// ABOUTME: Verifies the stored max_tool_iterations renders, edits propagate, bounds hold, clearing is explicit

import { describe, it, expect, vi } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import {
  MIN_MAX_TOOL_ITERATIONS,
  MAX_MAX_TOOL_ITERATIONS,
  DEFAULT_MAX_TOOL_ITERATIONS,
} from '@pierre/shared-constants';
import CoachFormModal from '../CoachFormModal';
import { DEFAULT_COACH_FORM_DATA, type CoachFormData } from '../coachForm';

function makeFormData(overrides: Partial<CoachFormData> = {}): CoachFormData {
  return {
    ...DEFAULT_COACH_FORM_DATA,
    title: 'Marathon Coach',
    system_prompt: 'You are an expert marathon coach.',
    ...overrides,
  };
}

function renderModal(
  formData: CoachFormData,
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
  it('renders the coach’s stored budget', () => {
    const { input } = renderModal(makeFormData({ max_tool_iterations: 27 }));

    expect(input.value).toBe('27');
    expect(input.min).toBe(String(MIN_MAX_TOOL_ITERATIONS));
    expect(input.max).toBe(String(MAX_MAX_TOOL_ITERATIONS));
  });

  it('leaves an untouched budget empty so the coach inherits the workspace limit', () => {
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
    const next = onFormDataChange.mock.calls[0][0] as CoachFormData;
    expect(next.max_tool_iterations).toBe(18);
    expect(next.title).toBe('Marathon Coach');
  });

  it('clamps a typed value above the ceiling down to the ceiling', () => {
    const { input, onFormDataChange } = renderModal(makeFormData({ max_tool_iterations: 10 }));

    fireEvent.change(input, { target: { value: '9000' } });

    const next = onFormDataChange.mock.calls[0][0] as CoachFormData;
    expect(next.max_tool_iterations).toBe(MAX_MAX_TOOL_ITERATIONS);
  });

  it('emptying the box on a pinned coach asks to clear, not to leave untouched', () => {
    const { input, onFormDataChange } = renderModal(makeFormData({ max_tool_iterations: 42 }));

    fireEvent.change(input, { target: { value: '' } });

    const next = onFormDataChange.mock.calls[0][0] as CoachFormData;
    // `null`, not `undefined`: undefined is the untouched state the request
    // omits, which would preserve the 42 the user just deleted.
    expect(next.max_tool_iterations).toBeNull();
  });

  it('renders an empty box for a coach whose pin was cleared', () => {
    const { input } = renderModal(makeFormData({ max_tool_iterations: null }));

    expect(input.value).toBe('');
  });
});

describe('CoachFormModal as the edit sheet', () => {
  it('is an edit form: no create-mode copy anywhere', () => {
    renderModal(makeFormData());

    expect(screen.getByRole('heading', { name: 'Edit Coach' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Save Changes' })).toBeInTheDocument();
    expect(screen.queryByText('Create Custom Coach')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Create Coach' })).not.toBeInTheDocument();
  });

  it('offers "Delete this coach" only when the mount owns deletion', () => {
    const onDelete = vi.fn();
    renderModal(makeFormData(), vi.fn(), { onDelete });

    fireEvent.click(screen.getByRole('button', { name: 'Delete this coach' }));

    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it('renders no delete affordance without an onDelete handler', () => {
    renderModal(makeFormData());

    expect(screen.queryByRole('button', { name: 'Delete this coach' })).not.toBeInTheDocument();
  });
});
