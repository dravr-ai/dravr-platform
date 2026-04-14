// ABOUTME: Sprint C8 tests — CoachNotesAuditTab renders, filters, and counts by scope
// ABOUTME: Mocks adminApi.listCoachNoteAudit and asserts content search + scope counters
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import CoachNotesAuditTab from '../CoachNotesAuditTab';
import type { CoachNoteAuditRow } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    listCoachNoteAudit: vi.fn(),
  },
}));

vi.mock('../../hooks/useAuth', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'admin@example.com',
      tenant_id: 'tenant-a',
      is_admin: true,
      role: 'admin',
      tier: 'professional',
      created_at: '2026-01-01T00:00:00Z',
    },
  }),
}));

const { adminApi } = await import('../../services/api/admin');

function sampleNote(overrides: Partial<CoachNoteAuditRow> = {}): CoachNoteAuditRow {
  return {
    id: 'note-1',
    tenant_id: 'tenant-a',
    user_id: 'user-42',
    coach_id: 'coach-strength',
    conversation_id: 'conv-7',
    scope: 'user',
    content: 'User prefers Monday morning strength sessions.',
    created_at: '2026-04-13T18:00:00Z',
    updated_at: '2026-04-13T18:00:00Z',
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <CoachNotesAuditTab />
    </QueryClientProvider>,
  );
}

describe('CoachNotesAuditTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no notes exist', async () => {
    vi.mocked(adminApi.listCoachNoteAudit).mockResolvedValueOnce({
      notes: [],
      total: 0,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/No coach notes match these filters/i)).toBeInTheDocument();
    });
  });

  it('renders notes with coach and user ids in the metadata row', async () => {
    vi.mocked(adminApi.listCoachNoteAudit).mockResolvedValueOnce({
      notes: [
        sampleNote(),
        sampleNote({
          id: 'note-2',
          coach_id: 'coach-endurance',
          user_id: 'user-7',
          scope: 'conversation',
          content: 'Mentioned achilles pain last session.',
        }),
      ],
      total: 2,
    });
    renderTab();
    await waitFor(() => {
      expect(
        screen.getByText(/User prefers Monday morning strength sessions/),
      ).toBeInTheDocument();
    });
    expect(screen.getByText(/Mentioned achilles pain last session/)).toBeInTheDocument();
    expect(screen.getByText(/coach coach-strength/)).toBeInTheDocument();
    expect(screen.getByText(/coach coach-endurance/)).toBeInTheDocument();
  });

  it('filters notes by content search', async () => {
    vi.mocked(adminApi.listCoachNoteAudit).mockResolvedValueOnce({
      notes: [
        sampleNote({
          id: 'note-1',
          content: 'User prefers Monday morning strength sessions.',
        }),
        sampleNote({
          id: 'note-2',
          content: 'Recovery days are Thursday and Sunday.',
        }),
      ],
      total: 2,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/User prefers/)).toBeInTheDocument();
    });
    const search = screen.getByPlaceholderText(/substring search/i);
    fireEvent.change(search, { target: { value: 'recovery' } });
    await waitFor(() => {
      expect(screen.queryByText(/User prefers/)).not.toBeInTheDocument();
    });
    expect(screen.getByText(/Recovery days/)).toBeInTheDocument();
  });

  it('filters notes by scope dropdown', async () => {
    vi.mocked(adminApi.listCoachNoteAudit).mockResolvedValueOnce({
      notes: [
        sampleNote({ id: 'note-1', scope: 'user', content: 'user scope note' }),
        sampleNote({
          id: 'note-2',
          scope: 'conversation',
          content: 'conversation scope note',
        }),
      ],
      total: 2,
    });
    renderTab();
    await waitFor(() => {
      expect(screen.getByText(/user scope note/)).toBeInTheDocument();
    });
    const scopeSelect = screen.getByRole('combobox', { name: /scope/i });
    fireEvent.change(scopeSelect, { target: { value: 'conversation' } });
    await waitFor(() => {
      expect(screen.queryByText(/user scope note/)).not.toBeInTheDocument();
    });
    expect(screen.getByText(/conversation scope note/)).toBeInTheDocument();
  });
});
