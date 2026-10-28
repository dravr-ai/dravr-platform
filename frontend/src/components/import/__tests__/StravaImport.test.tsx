// ABOUTME: Unit tests for the Strava import screen: link check and refusal, start, progress, completion, report, delete
// ABOUTME: The imports API is mocked; the copy is the shipped English catalogue, so a missing key shows up as a failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ArchiveImport } from '@pierre/shared-types';
import StravaImport from '../StravaImport';

const getCurrentImport = vi.fn();
const checkLink = vi.fn();
const startImport = vi.fn();
const cancelImport = vi.fn();
const getReport = vi.fn();
const deleteHistory = vi.fn();

vi.mock('../../../services/api', () => ({
  importsApi: {
    getCurrentImport: (...args: unknown[]) => getCurrentImport(...args),
    checkLink: (...args: unknown[]) => checkLink(...args),
    startImport: (...args: unknown[]) => startImport(...args),
    cancelImport: (...args: unknown[]) => cancelImport(...args),
    getReport: (...args: unknown[]) => getReport(...args),
    deleteHistory: (...args: unknown[]) => deleteHistory(...args),
  },
}));

function job(overrides: Partial<ArchiveImport>): ArchiveImport {
  return {
    id: 'job-1',
    source: 'strava_archive',
    status: 'completed',
    total_rows: 3,
    processed_rows: 3,
    imported: 2,
    skipped: 0,
    failed: 1,
    error_code: null,
    earliest_start: '2025-03-01T08:00:00Z',
    latest_start: '2026-03-01T08:00:00Z',
    created_at: '2026-10-27T08:00:00Z',
    updated_at: '2026-10-27T09:00:00Z',
    finished_at: '2026-10-27T09:00:00Z',
    expires_at: null,
    history_held: true,
    ...overrides,
  };
}

function renderImport() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <StravaImport enabled />
    </QueryClientProvider>,
  );
}

describe('StravaImport', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('checks a pasted link, says why a refused one is refused, and starts nothing', async () => {
    getCurrentImport.mockResolvedValue(null);
    checkLink.mockResolvedValue({ accepted: false, refusal: 'link_host_not_allowed', size_bytes: null });
    const user = userEvent.setup();
    renderImport();

    await user.type(await screen.findByTestId('import-link-input'), 'https://elsewhere.example.com/x.zip');
    await user.click(screen.getByTestId('import-check'));

    expect(await screen.findByTestId('import-error')).toHaveTextContent("That link isn't from Strava's export email.");
    expect(startImport).not.toHaveBeenCalled();
  });

  it('starts the import from a link the check accepted', async () => {
    getCurrentImport.mockResolvedValue(null);
    checkLink.mockResolvedValue({ accepted: true, refusal: null, size_bytes: 1_200_000_000 });
    startImport.mockResolvedValue(job({ status: 'queued', imported: 0, failed: 0 }));
    const user = userEvent.setup();
    renderImport();

    await user.type(await screen.findByTestId('import-link-input'), 'https://export.example.com/x.zip');
    await user.click(screen.getByTestId('import-check'));
    expect(await screen.findByTestId('import-link-found')).toHaveTextContent('Export found · about 1.2 GB');
    await user.click(screen.getByTestId('import-start'));

    await waitFor(() => expect(startImport).toHaveBeenCalledWith('strava_archive', 'https://export.example.com/x.zip'));
  });

  it('shows a running import with its progress and a cancel', async () => {
    getCurrentImport.mockResolvedValue(job({ status: 'importing', total_rows: 3105, processed_rows: 1240 }));
    cancelImport.mockResolvedValue(true);
    const user = userEvent.setup();
    renderImport();

    const running = await screen.findByTestId('import-running');
    expect(running).toHaveTextContent('1240 of 3105 activities read');
    await user.click(screen.getByText('Cancel import'));
    await waitFor(() => expect(cancelImport).toHaveBeenCalledWith('strava_archive', 'job-1'));
  });

  it('shows a completed import, its report, and deletes its history after a confirmation', async () => {
    getCurrentImport.mockResolvedValue(job({}));
    getReport.mockResolvedValue({
      issues: [{ row: 4, activity_id: '9003', outcome: 'missing_file', detail: null }],
      next_offset: null,
    });
    deleteHistory.mockResolvedValue(undefined);
    const user = userEvent.setup();
    renderImport();

    expect(await screen.findByTestId('import-completed')).toHaveTextContent('Activities imported: 2');
    await user.click(screen.getByText('Details'));
    expect(await screen.findByTestId('import-report')).toHaveTextContent('Its file is missing from the export');

    await user.click(screen.getByText('Delete imported history'));
    await user.click(await screen.findByText('Delete 2 activities'));
    await waitFor(() => expect(deleteHistory).toHaveBeenCalledWith('strava_archive'));
  });

  it('says why a failed import stopped, and asks for the link again', async () => {
    getCurrentImport.mockResolvedValue(job({ status: 'failed', error_code: 'link_expired', imported: 0 }));
    renderImport();

    expect(await screen.findByTestId('import-failed')).toHaveTextContent(
      'The link expired before the download finished.',
    );
    expect(screen.getByTestId('import-link-input')).toBeInTheDocument();
  });
});
