// ABOUTME: Tests for EvalHarnessTab fixture browser and Tier 5.5 verdict calibration panel
// ABOUTME: Mocks adminApi + useAuth and asserts summary counters, case drill-down, pass rate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import EvalHarnessTab from '../EvalHarnessTab';
import type {
  EvalFixtureBrowserResponse,
  VerdictCalibrationStats,
} from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    getEvalFixtureBrowser: vi.fn(),
    getVerdictCalibrationStats: vi.fn(),
    getEvalFixtureContent: vi.fn(),
    putEvalFixture: vi.fn(),
    deleteEvalFixture: vi.fn(),
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

function sampleCalibration(
  overrides: Partial<VerdictCalibrationStats> = {},
): VerdictCalibrationStats {
  return {
    window_start: '2026-03-14T12:00:00Z',
    window_days: 30,
    totals: {
      supported: 12,
      unsupported: 3,
      contradicted: 1,
      rhetorical: 4,
      unverifiable: 2,
    },
    daily: [
      {
        date: '2026-04-12',
        counts: {
          supported: 5,
          unsupported: 1,
          contradicted: 0,
          rhetorical: 1,
          unverifiable: 0,
        },
      },
      {
        date: '2026-04-13',
        counts: {
          supported: 7,
          unsupported: 2,
          contradicted: 1,
          rhetorical: 3,
          unverifiable: 2,
        },
      },
    ],
    ...overrides,
  };
}

function emptyCalibration(): VerdictCalibrationStats {
  return {
    window_start: '2026-03-14T12:00:00Z',
    window_days: 30,
    totals: {
      supported: 0,
      unsupported: 0,
      contradicted: 0,
      rhetorical: 0,
      unverifiable: 0,
    },
    daily: [],
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
    // Default: calibration returns an empty window so fixture-focused
    // tests don't also have to set it up.
    vi.mocked(adminApi.getVerdictCalibrationStats).mockResolvedValue(emptyCalibration());
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

  it('renders the verdict calibration panel with pass rate and status pills', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValueOnce(
      sampleResponse({ fixture_count: 0, case_total: 0, fixtures: [] }),
    );
    vi.mocked(adminApi.getVerdictCalibrationStats).mockResolvedValueOnce(
      sampleCalibration(),
    );
    renderTab();

    const panel = await screen.findByTestId('verdict-calibration-panel');
    // Pass rate = supported / (supported + unsupported + contradicted)
    //           = 12 / (12 + 3 + 1) = 75 %
    expect(
      within(panel).getByTestId('calibration-pass-rate'),
    ).toHaveTextContent('75%');
    expect(within(panel).getByText('Supported')).toBeInTheDocument();
    expect(within(panel).getByText('Contradicted')).toBeInTheDocument();
    expect(within(panel).getByTestId('calibration-daily-drift')).toBeInTheDocument();
  });

  it('shows an empty-window note when no verdicts exist in the calibration window', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValueOnce(
      sampleResponse({ fixture_count: 0, case_total: 0, fixtures: [] }),
    );
    vi.mocked(adminApi.getVerdictCalibrationStats).mockResolvedValueOnce(
      emptyCalibration(),
    );
    renderTab();

    const panel = await screen.findByTestId('verdict-calibration-panel');
    expect(
      within(panel).getByText(/No verdicts recorded in the selected window/i),
    ).toBeInTheDocument();
    // Pass rate falls back to the em dash when the denominator is zero.
    expect(
      within(panel).getByTestId('calibration-pass-rate'),
    ).toHaveTextContent('—');
  });

  it('opens the fixture editor with existing JSONL body when Edit is clicked', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValue(sampleResponse());
    vi.mocked(adminApi.getEvalFixtureContent).mockResolvedValueOnce({
      name: 'injury_triage',
      body: '{"id":"c1","label":"Knee","persona":"injury_coach","turns":[]}',
    });
    renderTab();

    await waitFor(() => {
      expect(screen.getByText('injury_triage')).toBeInTheDocument();
    });
    // Expand the fixture card to surface the Edit button.
    fireEvent.click(screen.getByRole('button', { name: /Toggle injury_triage fixture/i }));
    const editButton = await screen.findByRole('button', { name: /^Edit$/ });
    fireEvent.click(editButton);

    const modal = await screen.findByTestId('fixture-editor-modal');
    const textarea = within(modal).getByTestId('fixture-editor-body') as HTMLTextAreaElement;
    await waitFor(() => {
      expect(textarea.value).toContain('"id":"c1"');
    });
    expect(adminApi.getEvalFixtureContent).toHaveBeenCalledWith('injury_triage');
  });

  it('saves a new fixture through the editor and refreshes the browser list', async () => {
    vi.mocked(adminApi.getEvalFixtureBrowser).mockResolvedValue(
      sampleResponse({ fixture_count: 0, case_total: 0, fixtures: [] }),
    );
    vi.mocked(adminApi.putEvalFixture).mockResolvedValueOnce({
      name: 'new_fixture',
      path: '/tmp/new_fixture.jsonl',
      case_count: 1,
      personas: ['marathon_coach'],
      cases: [
        {
          id: 'c1',
          label: 'First',
          persona: 'marathon_coach',
          turn_count: 1,
          must_contain_total: 0,
          must_not_contain_total: 0,
        },
      ],
    });
    renderTab();

    fireEvent.click(await screen.findByRole('button', { name: /^New fixture$/ }));
    const modal = await screen.findByTestId('fixture-editor-modal');
    const nameInput = within(modal).getByLabelText(/Name/i) as HTMLInputElement;
    fireEvent.change(nameInput, { target: { value: 'new_fixture' } });
    const textarea = within(modal).getByTestId('fixture-editor-body') as HTMLTextAreaElement;
    fireEvent.change(textarea, {
      target: {
        value: '{"id":"c1","label":"First","persona":"marathon_coach","turns":[]}',
      },
    });

    fireEvent.click(within(modal).getByRole('button', { name: /Save fixture/i }));

    await waitFor(() => {
      expect(adminApi.putEvalFixture).toHaveBeenCalledWith(
        'new_fixture',
        '{"id":"c1","label":"First","persona":"marathon_coach","turns":[]}',
      );
    });
  });
});
