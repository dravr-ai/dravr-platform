// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for onboarding's Santé screen — an unusable question read puts the step off, once, after render
// ABOUTME: Pins the failed read and the read with no question list, which crashed the flow on CI (carnet#1530)

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import OnboardingParq from '../OnboardingParq';

const mockGetParqQuestions = vi.fn();

vi.mock('../../services/api', () => ({
  userApi: {
    getParqQuestions: () => mockGetParqQuestions(),
    submitParq: vi.fn(),
  },
}));

function renderScreen(onComplete = vi.fn()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingParq onComplete={onComplete} />
    </QueryClientProvider>,
  );
  return { onComplete };
}

describe('OnboardingParq', () => {
  let consoleError: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    mockGetParqQuestions.mockReset();
    consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
  });

  afterEach(() => {
    consoleError.mockRestore();
  });

  it('puts the step off once when the questions cannot be read, without updating its parent mid-render', async () => {
    mockGetParqQuestions.mockRejectedValue(new Error('network down'));
    const { onComplete } = renderScreen();

    await waitFor(() => expect(onComplete).toHaveBeenCalledWith('skipped'));
    expect(onComplete).toHaveBeenCalledTimes(1);
    const renderPhaseUpdate = consoleError.mock.calls.some((call: unknown[]) =>
      String(call[0]).includes('Cannot update a component'),
    );
    expect(renderPhaseUpdate, 'the step was put off during render').toBe(false);
  });

  it('puts the step off once when the read carries no question list', async () => {
    mockGetParqQuestions.mockResolvedValue({});
    const { onComplete } = renderScreen();

    await waitFor(() => expect(onComplete).toHaveBeenCalledWith('skipped'));
    expect(onComplete).toHaveBeenCalledTimes(1);
  });

  it('shows the questions and holds the step while they read fine', async () => {
    mockGetParqQuestions.mockResolvedValue({
      questions: [{ id: 'heart_condition', text: 'Has a doctor ever said you have a heart condition?' }],
    });
    const { onComplete } = renderScreen();

    expect(await screen.findByText('Has a doctor ever said you have a heart condition?')).toBeInTheDocument();
    expect(onComplete).not.toHaveBeenCalled();
  });
});
