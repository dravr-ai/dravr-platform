// ABOUTME: Sprint C16 tests — EvalHarnessTab renders fixture list, expands details
// ABOUTME: Mocks adminApi.getEvalFixtureBrowser and asserts summary counters + case drill-down
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import EvalHarnessTab from '../EvalHarnessTab';
import type { EvalFixtureBrowserResponse } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getEvalFixtureBrowser: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api/admin');

function sampleResponse(
  overrides: Partial<EvalFixtureBrowserResponse> = {},
): EvalFixtureBrowserResponse {
  return {
    scanned_dir: '/workspace/crates/pierre-evals/fixtures',
    fixture_count: 1,
    case_total: 2,
    fixtures: [
      {
        name: 'injury_triage',
        path: '/workspace/crates/pierre-evals/fixtures/injury_triage.jsonl',
        case_count: 2,
        personas: ['injury_coach'],
        cases: [
          {
            id: 'c1',
            label: 'Knee triage',
            persona: 'injury_coach',
            turn_count: 1,
            must_contain_total: 2,
            must_not_contain_total: 1,
          },
          {
            id: 'c2',
            label: 'Acute pain',
            persona: 'injury_coach',
            turn_count: 2,
            must_contain_total: 3,
            must_not_contain_total: 0,
          },
        ],
      },
    ],
    ...overrides,
  };
}

function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <EvalHarnessTab />
    </QueryClientProvider>,
  );
}

describe('EvalHarnessTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the empty state when no fixtures exist', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValueOnce(
      sampleResponse({ fixture_count: 0, case_total: 0, fixtures: [] }),
    );
    renderTab();
    await waitFor(() => {
      expect(
        screen.getByText(/No fixture files found in the scanned directory/i),
      ).toBeInTheDocument();
    });
  });

  it('renders fixture summary counts and the scanned directory', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('injury_triage')).toBeInTheDocument();
    });
    expect(screen.getByText(/\/workspace\/crates\/pierre-evals\/fixtures/)).toBeInTheDocument();
    // Cases are hidden until the fixture card is expanded.
    expect(screen.queryByText('Knee triage')).not.toBeInTheDocument();
  });

  it('expands a fixture card to show its case rows', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValueOnce(sampleResponse());
    renderTab();
    await waitFor(() => {
      expect(screen.getByText('injury_triage')).toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole('button', { name: /Toggle injury_triage fixture/i }));
    await waitFor(() => {
      expect(screen.getByText('Knee triage')).toBeInTheDocument();
    });
    expect(screen.getByText('Acute pain')).toBeInTheDocument();
  });

  it('renders the error state when the API call fails', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockRejectedValueOnce(
      new Error('Fixtures directory not found'),
    );
    renderTab();
    await waitFor(() => {
      expect(
        screen.getByText(/Failed to load eval fixtures/i),
      ).toBeInTheDocument();
    });
  });
});
