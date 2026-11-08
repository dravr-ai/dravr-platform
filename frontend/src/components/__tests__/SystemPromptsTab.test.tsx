// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The admin Prompts screen reads an agent prompt per language and says which language it shows
// ABOUTME: A fallback is stated rather than passed off as a translation, and system prompts read as English only

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import SystemPromptsTab from '../SystemPromptsTab';
import { adminApi } from '../../services/api';

vi.mock('../../services/api', () => ({
  adminApi: {
    getContremaitreStatus: vi.fn(),
    listSystemPrompts: vi.fn(),
    getSystemPrompt: vi.fn(),
    listAgentPrompts: vi.fn(),
    getAgentPrompt: vi.fn(),
    updateSystemPrompt: vi.fn(),
    triggerContremaitreSync: vi.fn(),
  },
}));

const viewer = vi.hoisted(() => ({ role: 'super_admin' }));
vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({ user: { role: viewer.role } }),
}));

const api = vi.mocked(adminApi);

function agentPrompt(requested: string, served: string) {
  return {
    key: 'endurance-agent',
    requested_locale: requested,
    locale: served,
    content: `text in ${served}`,
    sha256: 'abc',
    source: 'contremaitre' as const,
    loaded_at: '2026-11-06T12:00:00Z',
  };
}

function renderTab() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <SystemPromptsTab />
    </QueryClientProvider>,
  );
}

async function openAgent() {
  await userEvent.click(await screen.findByRole('button', { name: /Endurance Agent/ }));
}

describe('SystemPromptsTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    viewer.role = 'super_admin';
    api.getContremaitreStatus.mockResolvedValue({
      configured: true,
      repo: 'dravr-ai/dravr-contremaitre',
      branch: 'main',
      system_prompt_count: 22,
      agent_prompt_count: 75,
      persona_count: 4,
      compiled_in_count: 25,
      contremaitre_count: 76,
    });
    api.listSystemPrompts.mockResolvedValue({
      total: 1,
      prompts: [
        {
          key: 'conversation_summary',
          sha256: 'def',
          source: 'contremaitre',
          loaded_at: '2026-11-06T12:00:00Z',
          content_length: 12,
        },
      ],
    });
    api.getSystemPrompt.mockResolvedValue({
      key: 'conversation_summary',
      content: 'Summarize.',
      sha256: 'def',
      source: 'contremaitre',
      loaded_at: '2026-11-06T12:00:00Z',
    });
    api.listAgentPrompts.mockResolvedValue({
      total: 1,
      prompts: [{ key: 'endurance-agent', locales: ['en', 'es', 'fr'] }],
    });
    api.getAgentPrompt.mockImplementation(async (_slug, locale) =>
      agentPrompt(locale, locale === 'de' ? 'fr' : locale),
    );
  });

  it('says the agent prompts could not be read rather than counting none', async () => {
    api.listAgentPrompts.mockRejectedValue(new Error('403'));
    renderTab();

    expect(await screen.findByText('Agent prompts could not be loaded.')).toBeInTheDocument();
    expect(screen.queryByText('0 Agent Prompts')).not.toBeInTheDocument();
  });

  it('says the system prompts could not be read rather than counting none', async () => {
    api.listSystemPrompts.mockRejectedValue(new Error('403'));
    renderTab();

    expect(await screen.findByText('System prompts could not be loaded.')).toBeInTheDocument();
    expect(screen.queryByText('0 System Prompts')).not.toBeInTheDocument();
  });

  it('offers every supported language and reads the one chosen', async () => {
    renderTab();
    await openAgent();
    const languages = screen.getByRole('group', { name: 'Prompt language' });
    expect(languages.querySelectorAll('button')).toHaveLength(5);
    expect(await screen.findByText('text in en')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Español' }));

    expect(api.getAgentPrompt).toHaveBeenLastCalledWith('endurance-agent', 'es');
    expect(await screen.findByText('text in es')).toBeInTheDocument();
    expect(screen.getByText('Showing the Español prompt.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Español' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('states a fallback and names the language actually shown', async () => {
    renderTab();
    await openAgent();
    await userEvent.click(screen.getByRole('button', { name: 'Deutsch' }));

    expect(await screen.findByText('text in fr')).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent(
      'No Deutsch translation is loaded. Showing the Français prompt',
    );
    expect(screen.queryByText('Showing the Deutsch prompt.')).not.toBeInTheDocument();
  });

  it('says when a language has nothing to show, and offers no editor on an agent', async () => {
    api.getAgentPrompt.mockRejectedValue(new Error('not found'));
    renderTab();
    await openAgent();

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'This prompt could not be loaded in English.',
    );
    expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
  });

  it('marks system prompts as English only and offers them no language control', async () => {
    renderTab();
    expect(await screen.findByText(/Written in English only/)).toBeInTheDocument();

    await userEvent.click(await screen.findByRole('button', { name: /Conversation Summary/ }));

    expect(await screen.findByText('Summarize.')).toBeInTheDocument();
    expect(screen.getByText('English only')).toBeInTheDocument();
    expect(screen.queryByRole('group', { name: 'Prompt language' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Edit' })).toBeInTheDocument();
  });

  it('shows a plain admin the agent prompts and asks for nothing the server refuses them', async () => {
    viewer.role = 'admin';
    api.getAgentPrompt.mockResolvedValue(agentPrompt('en', 'en'));
    renderTab();

    await openAgent();
    expect(await screen.findByText('text in en')).toBeTruthy();

    expect(api.getContremaitreStatus).not.toHaveBeenCalled();
    expect(api.listSystemPrompts).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: 'Sync Now' })).toBeNull();
    expect(screen.queryByText('System prompts could not be loaded.')).toBeNull();
    expect(screen.getByText(/System prompts and sync are for super admins/)).toBeTruthy();
  });

  it('states counts that add up and words for what each one means', async () => {
    renderTab();

    expect((await screen.findByTestId('prompt-counts')).textContent).toBe(
      '101 prompts: 22 system, 75 agent per language, 4 persona. 76 synced from contremaitre, 25 seed copies.',
    );
    expect(screen.getByText('Sync configured')).toBeTruthy();
    expect(screen.queryByText('Connected')).toBeNull();
    expect(await screen.findByText('synced')).toBeTruthy();
  });
});
