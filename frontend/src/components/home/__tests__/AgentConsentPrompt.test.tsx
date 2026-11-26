// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for Home's one-time question letting the athlete's agent use its memory and read their health records
// ABOUTME: Shown only while due; both boxes start unticked; Save sends the ticked ones, Not now dismisses, a given class is not re-offered

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AgentConsentPrompt } from '../AgentConsentPrompt';

const getAgentConsentPrompt = vi.fn();
const answerAgentConsentPrompt = vi.fn();

vi.mock('../../../services/api', () => ({
  oauthApi: {
    getAgentConsentPrompt: (...a: unknown[]) => getAgentConsentPrompt(...a),
    answerAgentConsentPrompt: (...a: unknown[]) => answerAgentConsentPrompt(...a),
  },
}));

vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

type State = { due: boolean; memory: boolean; medical: boolean };

function renderPrompt() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <AgentConsentPrompt />
    </QueryClientProvider>,
  );
}

describe('AgentConsentPrompt', () => {
  let state: State;

  beforeEach(() => {
    vi.clearAllMocks();
    state = { due: true, memory: false, medical: false };
    getAgentConsentPrompt.mockImplementation(async () => state);
    answerAgentConsentPrompt.mockImplementation(async (answer: Record<string, boolean>) => {
      state = {
        due: false,
        memory: state.memory || answer.memory === true,
        medical: state.medical || answer.medical === true,
      };
      return state;
    });
  });

  it('asks with both boxes unticked, Law 25 opt-in', async () => {
    renderPrompt();
    expect(await screen.findByTestId('agent-consent-prompt')).toBeInTheDocument();
    expect(screen.getByText('Should your agent remember you?')).toBeInTheDocument();
    expect(screen.getByTestId('agent-consent-prompt-memory')).not.toBeChecked();
    expect(screen.getByTestId('agent-consent-prompt-medical')).not.toBeChecked();
  });

  it('saves only the ticked consent and is not shown again', async () => {
    renderPrompt();
    await userEvent.click(await screen.findByTestId('agent-consent-prompt-memory'));
    await userEvent.click(screen.getByTestId('agent-consent-prompt-save'));
    expect(answerAgentConsentPrompt).toHaveBeenCalledWith({ memory: true, medical: false });
    await waitFor(() => expect(screen.queryByTestId('agent-consent-prompt')).not.toBeInTheDocument());
    expect(screen.getByTestId('agent-consent-prompt-saved')).toHaveTextContent('Saved.');
  });

  it('dismisses with Not now, giving nothing', async () => {
    renderPrompt();
    await userEvent.click(await screen.findByTestId('agent-consent-prompt-memory'));
    await userEvent.click(screen.getByTestId('agent-consent-prompt-not-now'));
    expect(answerAgentConsentPrompt).toHaveBeenCalledWith({ dismissed: true });
    await waitFor(() => expect(screen.queryByTestId('agent-consent-prompt')).not.toBeInTheDocument());
    expect(state).toEqual({ due: false, memory: false, medical: false });
    // Nothing was saved, so nothing says it was.
    expect(screen.queryByTestId('agent-consent-prompt-saved')).not.toBeInTheDocument();
  });

  it('offers only the class not yet given', async () => {
    state = { due: true, memory: false, medical: true };
    renderPrompt();
    expect(await screen.findByTestId('agent-consent-prompt-memory')).toBeInTheDocument();
    expect(screen.queryByTestId('agent-consent-prompt-medical')).not.toBeInTheDocument();
  });

  it('shows nothing once answered', async () => {
    state = { due: false, memory: false, medical: false };
    renderPrompt();
    await waitFor(() => expect(getAgentConsentPrompt).toHaveBeenCalled());
    expect(screen.queryByTestId('agent-consent-prompt')).not.toBeInTheDocument();
  });

  it('says so when the answer could not be saved, and stays', async () => {
    answerAgentConsentPrompt.mockRejectedValueOnce(new Error('down'));
    renderPrompt();
    await userEvent.click(await screen.findByTestId('agent-consent-prompt-save'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Your choice could not be saved.');
    expect(screen.getByTestId('agent-consent-prompt')).toBeInTheDocument();
  });
});
