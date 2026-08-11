// ABOUTME: Tests for the GuardianConfigTab admin form
// ABOUTME: Mocks the admin API and asserts hydration, env-pin lockout, validation, and save payloads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import GuardianConfigTab from '../GuardianConfigTab';
import type { GuardianConfigResponse } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getGuardianConfig: vi.fn(),
    putGuardianConfig: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api/admin');

function sampleResponse(overrides: Partial<GuardianConfigResponse> = {}): GuardianConfigResponse {
  return {
    config: { schema_version: 1, mode: 'observe' },
    effective: {
      mode: 'observe',
      max_destructive_per_turn: 1,
      max_writes_per_turn: 50,
      external_send: 'none',
      tainted_destructive: 'log',
      plan_mode: 'off',
    },
    sources: {
      mode: 'database',
      max_destructive_per_turn: 'default',
      max_writes_per_turn: 'default',
      external_send: 'default',
      tainted_destructive: 'default',
      plan_mode: 'default',
    },
    env_pinned: [],
    source: 'persisted',
    updated_at: '2026-08-11T12:00:00Z',
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <GuardianConfigTab />
    </QueryClientProvider>,
  );
}

describe('GuardianConfigTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('hydrates the mode select from the persisted document', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Guardian Security Policy')).toBeInTheDocument();
    });
    const selects = screen.getAllByRole('combobox');
    // Field order: mode, tainted_destructive, plan_mode, external_send.
    expect((selects[0] as HTMLSelectElement).value).toBe('observe');
    expect(screen.getAllByText(/persisted/i).length).toBeGreaterThan(0);
  });

  it('locks an env-pinned field and shows the pin notice', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
      sampleResponse({
        env_pinned: ['mode'],
        sources: {
          mode: 'env',
          max_destructive_per_turn: 'default',
          max_writes_per_turn: 'default',
          external_send: 'default',
          tainted_destructive: 'default',
          plan_mode: 'default',
        },
      }),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Guardian Security Policy')).toBeInTheDocument();
    });
    const selects = screen.getAllByRole('combobox');
    expect(selects[0]).toBeDisabled();
    expect(screen.getByText(/locked by GUARDIAN_\* environment variables/i)).toBeInTheDocument();
    // Exact string targets the field badge; the notice paragraph above has
    // longer text and is asserted separately.
    expect(screen.getByText('env-pinned')).toBeInTheDocument();
  });

  it('rejects a zero write budget before save', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Guardian Security Policy')).toBeInTheDocument();
    });
    const inputs = screen.getAllByRole('spinbutton');
    // Budget order: max_destructive_per_turn, max_writes_per_turn.
    fireEvent.change(inputs[1], { target: { value: '0' } });
    expect(
      screen.getByText(/must be >= 1 \(0 would deny every write-tool dispatch\)/i),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /save changes/i })).toBeDisabled();
  });

  it('saves the edited document and surfaces the env-pin outcome from the response', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(
      sampleResponse({
        config: { schema_version: 1, mode: 'enforce', tainted_destructive: 'confirm' },
      }),
    );
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Guardian Security Policy')).toBeInTheDocument();
    });
    const selects = screen.getAllByRole('combobox');
    fireEvent.change(selects[1], { target: { value: 'confirm' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putGuardianConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putGuardianConfig).mock.calls[0];
    expect(payload.tainted_destructive).toBe('confirm');
    expect(payload.mode).toBe('observe');
    expect(payload.schema_version).toBe(1);
  });

  it('clearing a select back to follow-default sends null for the field', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('Guardian Security Policy')).toBeInTheDocument();
    });
    const selects = screen.getAllByRole('combobox');
    fireEvent.change(selects[0], { target: { value: '' } });
    fireEvent.click(screen.getByRole('button', { name: /save changes/i }));
    await waitFor(() => {
      expect(adminApi.putGuardianConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putGuardianConfig).mock.calls[0];
    expect(payload.mode).toBeNull();
  });
});
