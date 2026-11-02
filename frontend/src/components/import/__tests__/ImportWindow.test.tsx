// ABOUTME: Unit tests for the /import window's source list: a row per offered source, its state line, and opening one
// ABOUTME: The imports API is mocked; the copy is the shipped English catalogue, so a missing key shows up as a failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ArchiveImport, ImportsOverview } from '@pierre/shared-types';
import ImportWindow from '../ImportWindow';

const getImports = vi.fn();
const getCurrentImport = vi.fn();

vi.mock('../../../services/api', () => ({
  importsApi: {
    getImports: (...args: unknown[]) => getImports(...args),
    getCurrentImport: (...args: unknown[]) => getCurrentImport(...args),
  },
}));

function waiting(source: string): ArchiveImport {
  return {
    id: `job-${source}`,
    source,
    status: 'awaiting_link',
    total_rows: null,
    processed_rows: 0,
    imported: 0,
    skipped: 0,
    failed: 0,
    error_code: null,
    earliest_start: null,
    latest_start: null,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    finished_at: null,
    expires_at: null,
    history_held: false,
  };
}

function renderWindow(overview: ImportsOverview) {
  getImports.mockResolvedValue(overview);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ImportWindow onClose={() => undefined} />
    </QueryClientProvider>,
  );
}

describe('ImportWindow', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.history.replaceState(null, '', '/import');
  });

  // carnet#1337: the Garmin export was built but never offered.
  it('lists a row for each offered source, Garmin beside Strava, each with its own state line', async () => {
    renderWindow({
      enabled: true,
      sources: ['strava_archive', 'garmin_archive'],
      imports: [waiting('garmin_archive')],
    });

    expect(await screen.findByTestId('import-source-strava_archive')).toHaveTextContent('Strava');
    const garmin = screen.getByTestId('import-source-garmin_archive');
    expect(garmin).toHaveTextContent('Garmin Connect');
    expect(garmin).toHaveTextContent("Your activities and daily wellness from Garmin's data export");
    expect(screen.getByTestId('import-state-garmin_archive')).toHaveTextContent(
      "Waiting for Garmin's email · Paste the link",
    );
    expect(screen.queryByTestId('import-state-strava_archive')).not.toBeInTheDocument();
  });

  it('opens the Garmin import from its row, under its own hash', async () => {
    getCurrentImport.mockResolvedValue(null);
    const user = userEvent.setup();
    renderWindow({ enabled: true, sources: ['strava_archive', 'garmin_archive'], imports: [] });

    await user.click(await screen.findByTestId('import-source-garmin_archive'));

    expect(await screen.findByTestId('source-import-garmin_archive')).toBeInTheDocument();
    expect(window.location.hash).toBe('#garmin');
    expect(getCurrentImport).toHaveBeenCalledWith('garmin_archive');
  });

  it('shows no row for a source this build has no copy for', async () => {
    renderWindow({ enabled: true, sources: ['strava_archive', 'fitbit_archive'], imports: [] });

    expect(await screen.findByTestId('import-source-strava_archive')).toBeInTheDocument();
    expect(screen.queryByTestId('import-source-fitbit_archive')).not.toBeInTheDocument();
  });
});
