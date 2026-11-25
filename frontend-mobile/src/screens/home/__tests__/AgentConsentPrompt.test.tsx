// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for Home's one-time question on the phone letting the athlete's agent use its memory and read their health records
// ABOUTME: Shown only while due; both boxes start unticked; Save sends the ticked ones, Not now dismisses, a given class is not re-offered

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { AgentConsentPrompt } from '../AgentConsentPrompt';
import { oauthApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  oauthApi: {
    getAgentConsentPrompt: jest.fn(),
    answerAgentConsentPrompt: jest.fn(),
  },
}));
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true }),
}));

const getAgentConsentPrompt = oauthApi.getAgentConsentPrompt as jest.Mock;
const answerAgentConsentPrompt = oauthApi.answerAgentConsentPrompt as jest.Mock;

type State = { due: boolean; memory: boolean; medical: boolean };

function renderPrompt() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <AgentConsentPrompt />
    </QueryClientProvider>,
  );
}

describe('AgentConsentPrompt', () => {
  let state: State;

  beforeEach(() => {
    jest.clearAllMocks();
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
    expect(await screen.findByTestId('agent-consent-prompt')).toBeTruthy();
    expect(screen.getByText('Should your agent remember you?')).toBeTruthy();
    expect(screen.getByTestId('agent-consent-prompt-memory').props.accessibilityState).toEqual({ checked: false });
    expect(screen.getByTestId('agent-consent-prompt-medical').props.accessibilityState).toEqual({ checked: false });
  });

  it('saves only the ticked consent and is not shown again', async () => {
    renderPrompt();
    fireEvent.press(await screen.findByTestId('agent-consent-prompt-medical'));
    fireEvent.press(screen.getByTestId('agent-consent-prompt-save'));
    await waitFor(() => expect(answerAgentConsentPrompt).toHaveBeenCalledWith({ memory: false, medical: true }));
    await waitFor(() => expect(screen.queryByTestId('agent-consent-prompt')).toBeNull());
    expect(screen.getByTestId('agent-consent-prompt-saved')).toBeTruthy();
  });

  it('dismisses with Not now, giving nothing', async () => {
    renderPrompt();
    fireEvent.press(await screen.findByTestId('agent-consent-prompt-memory'));
    fireEvent.press(screen.getByTestId('agent-consent-prompt-not-now'));
    await waitFor(() => expect(answerAgentConsentPrompt).toHaveBeenCalledWith({ dismissed: true }));
    await waitFor(() => expect(screen.queryByTestId('agent-consent-prompt')).toBeNull());
    expect(state).toEqual({ due: false, memory: false, medical: false });
  });

  it('offers only the class not yet given', async () => {
    state = { due: true, memory: true, medical: false };
    renderPrompt();
    expect(await screen.findByTestId('agent-consent-prompt-medical')).toBeTruthy();
    expect(screen.queryByTestId('agent-consent-prompt-memory')).toBeNull();
  });

  it('shows nothing once answered', async () => {
    state = { due: false, memory: false, medical: false };
    renderPrompt();
    await waitFor(() => expect(getAgentConsentPrompt).toHaveBeenCalled());
    expect(screen.queryByTestId('agent-consent-prompt')).toBeNull();
  });

  it('says so when the answer could not be saved, and stays', async () => {
    answerAgentConsentPrompt.mockRejectedValueOnce(new Error('down'));
    renderPrompt();
    fireEvent.press(await screen.findByTestId('agent-consent-prompt-save'));
    expect(await screen.findByTestId('agent-consent-prompt-failed')).toBeTruthy();
    expect(screen.getByTestId('agent-consent-prompt')).toBeTruthy();
  });
});
