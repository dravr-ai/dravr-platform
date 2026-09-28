// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the agent edit sheet's version history — the list, the comparison with the current content, and revert
// ABOUTME: The coaches API is mocked at the services barrel; assertions read the rendered versions, changed fields and calls

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { formatDateTime } from '@pierre/chat-utils';
import type {
  Agent,
  AgentVersionDiffResponse,
  ListAgentVersionsResponse,
  RevertAgentVersionResponse,
} from '@pierre/shared-types';
import CoachVersionHistory from '../CoachVersionHistory';

const listVersions = vi.fn();
const diffVersion = vi.fn();
const revertToVersion = vi.fn();

vi.mock('../../../services/api', () => ({
  coachesApi: {
    listVersions: (...a: unknown[]) => listVersions(...a),
    diffVersion: (...a: unknown[]) => diffVersion(...a),
    revertToVersion: (...a: unknown[]) => revertToVersion(...a),
  },
}));

const AGENT_ID = 'coach-tempo';

function snapshot(title: string, prompt: string) {
  return {
    title,
    description: null,
    system_prompt: prompt,
    category: 'training',
    tags: ['tempo'],
    sample_prompts: [],
    token_count: 10,
    visibility: 'private',
  };
}

const HISTORY: ListAgentVersionsResponse = {
  versions: [
    {
      version: 2,
      content_snapshot: snapshot('Tempo Coach B', 'prompt two'),
      change_summary: null,
      created_at: '2026-09-20T08:30:00Z',
      created_by_name: 'Ada Lovelace',
    },
    {
      version: 1,
      content_snapshot: snapshot('Tempo Coach', 'prompt one'),
      change_summary: null,
      created_at: '2026-09-10T08:30:00Z',
      created_by_name: null,
    },
  ],
  current_version: 2,
  total: 2,
};

const DIFF_V1: AgentVersionDiffResponse = {
  version: 1,
  changes: [
    { field: 'title', old_value: 'Tempo Coach', new_value: 'Tempo Coach C' },
    { field: 'category', old_value: 'training', new_value: 'recovery' },
  ],
};

function revertedAgent(): Agent {
  return {
    id: AGENT_ID,
    title: 'Tempo Coach',
    description: null,
    system_prompt: 'prompt one',
    category: 'Training',
    tags: ['tempo'],
    token_count: 10,
    is_favorite: false,
    use_count: 0,
    last_used_at: null,
    created_at: '2026-09-01T00:00:00Z',
    updated_at: '2026-09-27T00:00:00Z',
    is_system: false,
    visibility: 'private',
  } as Agent;
}

function renderHistory(onReverted = vi.fn()) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <CoachVersionHistory agentId={AGENT_ID} onReverted={onReverted} />
    </QueryClientProvider>,
  );
  return { onReverted, queryClient };
}

describe('CoachVersionHistory', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    listVersions.mockResolvedValue(HISTORY);
    diffVersion.mockResolvedValue(DIFF_V1);
    const reverted: RevertAgentVersionResponse = {
      agent: revertedAgent(),
      reverted_to_version: 1,
      new_version: 3,
    };
    revertToVersion.mockResolvedValue(reverted);
  });

  it('lists every version the API returns, newest first, with its author when known', async () => {
    renderHistory();

    const newest = await screen.findByTestId('agent-version-2');
    const oldest = screen.getByTestId('agent-version-1');
    expect(listVersions).toHaveBeenCalledWith(AGENT_ID);
    expect(newest.compareDocumentPosition(oldest) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(newest).getByText(i18n.t('discover.versionLabel', { version: 2 }))).toBeInTheDocument();
    expect(within(newest).getByTestId('agent-version-meta')).toHaveTextContent(
      i18n.t('discover.versionReplacedBy', {
        date: formatDateTime(HISTORY.versions[0].created_at, i18n.language),
        author: 'Ada Lovelace',
      }),
    );
    // No recorded author: the line carries the date alone.
    expect(within(oldest).getByTestId('agent-version-meta')).toHaveTextContent(
      i18n.t('discover.versionReplaced', {
        date: formatDateTime(HISTORY.versions[1].created_at, i18n.language),
      }),
    );
  });

  it('says so when the agent has never been edited', async () => {
    listVersions.mockResolvedValue({ versions: [], current_version: 0, total: 0 });
    renderHistory();

    expect(await screen.findByTestId('agent-version-history-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('agent-version-1')).not.toBeInTheDocument();
  });

  it('compares a version with the current content and shows each changed field', async () => {
    const user = userEvent.setup();
    renderHistory();

    await user.click(await screen.findByTestId('agent-version-compare-1'));

    const diff = await screen.findByTestId('agent-version-diff-1');
    expect(diffVersion).toHaveBeenCalledWith(AGENT_ID, 1);
    const title = within(diff).getByTestId('agent-version-change-title');
    expect(within(title).getByTestId('agent-version-old-value')).toHaveTextContent('Tempo Coach');
    expect(within(title).getByTestId('agent-version-new-value')).toHaveTextContent('Tempo Coach C');
    // Enum values are named for the athlete, never shown raw.
    const category = within(diff).getByTestId('agent-version-change-category');
    expect(within(category).getByTestId('agent-version-old-value')).toHaveTextContent('Training');
    expect(within(category).getByTestId('agent-version-new-value')).toHaveTextContent('Recovery');
    // The untouched fields are not listed.
    expect(within(diff).queryByTestId('agent-version-change-system_prompt')).not.toBeInTheDocument();
  });

  it('reverts only after confirmation, then refreshes the history and hands back the agent', async () => {
    const user = userEvent.setup();
    const { onReverted } = renderHistory();

    await user.click(await screen.findByTestId('agent-version-revert-1'));
    expect(revertToVersion).not.toHaveBeenCalled();

    const dialog = screen.getByRole('dialog');
    await user.click(within(dialog).getByRole('button', { name: i18n.t('discover.versionRevertConfirm') }));

    await waitFor(() => expect(revertToVersion).toHaveBeenCalledWith(AGENT_ID, 1));
    await waitFor(() => expect(onReverted).toHaveBeenCalledWith(revertedAgent()));
    // The history is fetched again after the revert (initial load + refresh).
    await waitFor(() => expect(listVersions).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
  });

  it('cancelling the confirmation reverts nothing', async () => {
    const user = userEvent.setup();
    const { onReverted } = renderHistory();

    await user.click(await screen.findByTestId('agent-version-revert-2'));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: i18n.t('common.cancel') }));

    expect(revertToVersion).not.toHaveBeenCalled();
    expect(onReverted).not.toHaveBeenCalled();
  });
});
