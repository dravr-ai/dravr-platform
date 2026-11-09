// ABOUTME: Unit tests for a source's import screen, Strava's and Garmin's: link check and refusal, start, progress, report, delete
// ABOUTME: The imports API is mocked; the copy is the shipped English catalogue, so a missing key shows up as a failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ArchiveImport } from '@pierre/shared-types';
import { importSourceCopy, type ImportSourceCopy } from '@pierre/ui-logic';
import SourceImport from '../SourceImport';

function copyOf(source: string): ImportSourceCopy {
  const copy = importSourceCopy(source);
  if (!copy) throw new Error(`no copy for ${source}`);
  return copy;
}

const getCurrentImport = vi.fn();
const checkLink = vi.fn();
const startImport = vi.fn();
const cancelImport = vi.fn();
const getReport = vi.fn();
const deleteHistory = vi.fn();
const startUpload = vi.fn();
const getUploadSession = vi.fn();
const writeUpload = vi.fn();
const finishUpload = vi.fn();

vi.mock('../../../services/api', () => ({
  importsApi: {
    getCurrentImport: (...args: unknown[]) => getCurrentImport(...args),
    checkLink: (...args: unknown[]) => checkLink(...args),
    startImport: (...args: unknown[]) => startImport(...args),
    cancelImport: (...args: unknown[]) => cancelImport(...args),
    getReport: (...args: unknown[]) => getReport(...args),
    deleteHistory: (...args: unknown[]) => deleteHistory(...args),
    startUpload: (...args: unknown[]) => startUpload(...args),
    getUploadSession: (...args: unknown[]) => getUploadSession(...args),
    writeUpload: (...args: unknown[]) => writeUpload(...args),
    finishUpload: (...args: unknown[]) => finishUpload(...args),
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

function renderImport(source = 'strava_archive') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <SourceImport copy={copyOf(source)} enabled />
    </QueryClientProvider>,
  );
}

describe('SourceImport', () => {
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

  // carnet#1212: the history line and the confirmation had one plural form,
  // « 1 activities · imported … » and « Delete 1 activities ».
  it('counts a single imported activity in the singular, in the history and the confirmation', async () => {
    getCurrentImport.mockResolvedValue(job({ imported: 1 }));
    getReport.mockResolvedValue({ issues: [], next_offset: null });
    const user = userEvent.setup();
    renderImport();

    expect(await screen.findByTestId('import-completed')).toHaveTextContent(/1 activity · imported /);
    await user.click(screen.getByText('Delete imported history'));
    expect(
      await screen.findByText(
        "Delete 1 imported activity, and the routes and best efforts calculated from it? This can't be undone.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText('Delete 1 activity')).toBeInTheDocument();
  });

  it('says why a failed import stopped, and asks for the link again', async () => {
    getCurrentImport.mockResolvedValue(job({ status: 'failed', error_code: 'link_expired', imported: 0 }));
    renderImport();

    expect(await screen.findByTestId('import-failed')).toHaveTextContent(
      'The link expired before the download finished.',
    );
    expect(screen.getByTestId('import-link-input')).toBeInTheDocument();
  });

  // carnet#1337: the Garmin export is offered beside Strava's.
  it('tells the athlete where Garmin Connect makes the export, and that its link expires', async () => {
    getCurrentImport.mockResolvedValue(null);
    renderImport('garmin_archive');

    const view = await screen.findByTestId('source-import-garmin_archive');
    expect(screen.getByRole('heading', { level: 2 })).toHaveTextContent('Garmin Connect');
    expect(view).toHaveTextContent('Data Management, then Export Your Data');
    expect(view).toHaveTextContent('can take several days');
    expect(view).toHaveTextContent('It works for 3 days.');
    expect(screen.getByRole('link', { name: /Open Garmin's export page/ })).toHaveAttribute(
      'href',
      'https://www.garmin.com/account/datamanagement/exportdata/',
    );
    expect(view).not.toHaveTextContent('Strava');
    expect(getCurrentImport).toHaveBeenCalledWith('garmin_archive');
  });

  it('checks and starts a Garmin import under the Garmin source, naming Garmin when it refuses', async () => {
    getCurrentImport.mockResolvedValue(null);
    checkLink
      .mockResolvedValueOnce({ accepted: false, refusal: 'link_host_not_allowed', size_bytes: null })
      .mockResolvedValueOnce({ accepted: true, refusal: null, size_bytes: 200_717_053 });
    startImport.mockResolvedValue(job({ source: 'garmin_archive', status: 'queued', imported: 0, failed: 0 }));
    const user = userEvent.setup();
    renderImport('garmin_archive');

    const input = await screen.findByTestId('import-link-input');
    await user.type(input, 'https://elsewhere.example.com/x.zip');
    await user.click(screen.getByTestId('import-check'));
    expect(await screen.findByTestId('import-error')).toHaveTextContent("That link isn't from Garmin's export email.");
    expect(checkLink).toHaveBeenCalledWith('garmin_archive', 'https://elsewhere.example.com/x.zip');

    const link = 'https://s3.amazonaws.com/it-gdpr-bucket/0b6f3d8e_1.zip?X-Amz-Expires=259200';
    await user.clear(input);
    await user.type(input, link);
    await user.click(screen.getByTestId('import-check'));
    expect(await screen.findByTestId('import-link-found')).toHaveTextContent('Export found · about 200.7 MB');
    await user.click(screen.getByTestId('import-start'));

    await waitFor(() => expect(startImport).toHaveBeenCalledWith('garmin_archive', link));
  });
});

describe('SourceImport, an export the athlete uploads', () => {
  const SESSION = { kind: 'session', url: 'https://storage.example.com/session-secret' };
  const CHUNK_RANGE = /^bytes \d+-\d+\/\d+$/;

  /** A store's session: asked where it stands first, then takes every chunk. */
  function sessionFetch() {
    return vi.fn(async (_url: string, init: RequestInit) => {
      const range = (init.headers as Record<string, string>)['Content-Range'];
      return {
        status: CHUNK_RANGE.test(range) ? 201 : 308,
        headers: { get: () => null },
      } as unknown as Response;
    });
  }

  function exportFile() {
    return new File(['PK-export'], 'export.zip', { type: 'application/zip' });
  }

  beforeEach(() => {
    vi.clearAllMocks();
    vi.unstubAllGlobals();
  });

  it('offers a file, never a link, and opens no export page', async () => {
    getCurrentImport.mockResolvedValue(null);
    renderImport('apple_health_archive');

    expect(await screen.findByTestId('import-upload-pick')).toBeInTheDocument();
    expect(screen.queryByTestId('import-link-input')).not.toBeInTheDocument();
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
  });

  it('opens an upload of the file, sends it to the session and says it landed', async () => {
    getCurrentImport.mockResolvedValue(null);
    startUpload.mockResolvedValue({ import: job({ id: 'job-9', status: 'awaiting_link' }), upload: SESSION });
    finishUpload.mockResolvedValue(job({ id: 'job-9', status: 'downloading' }));
    const fetchMock = sessionFetch();
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    renderImport('apple_health_archive');

    await user.upload(await screen.findByTestId('import-upload-pick-input'), exportFile());

    await waitFor(() => expect(finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-9'));
    expect(startUpload).toHaveBeenCalledWith('apple_health_archive', 9);
    expect(fetchMock.mock.calls.map(([, init]) => (init.headers as Record<string, string>)['Content-Range'])).toEqual([
      'bytes */9',
      'bytes 0-8/9',
    ]);
  });

  it('resumes the import that waits rather than opening another', async () => {
    getCurrentImport.mockResolvedValue(job({ id: 'job-7', status: 'awaiting_link', imported: 0 }));
    getUploadSession.mockResolvedValue(SESSION);
    finishUpload.mockResolvedValue(job({ id: 'job-7', status: 'downloading' }));
    vi.stubGlobal('fetch', sessionFetch());
    const user = userEvent.setup();
    renderImport('apple_health_archive');

    await user.upload(await screen.findByTestId('import-upload-pick-input'), exportFile());

    await waitFor(() => expect(finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-7'));
    expect(getUploadSession).toHaveBeenCalledWith('apple_health_archive', 'job-7');
    expect(startUpload).not.toHaveBeenCalled();
  });

  it('says a store session that is gone cannot continue, and does not finish', async () => {
    getCurrentImport.mockResolvedValue(job({ id: 'job-7', status: 'awaiting_link', imported: 0 }));
    getUploadSession.mockResolvedValue(SESSION);
    vi.stubGlobal('fetch', vi.fn(async () => ({ status: 404, headers: { get: () => null } })));
    const user = userEvent.setup();
    renderImport('apple_health_archive');

    await user.upload(await screen.findByTestId('import-upload-pick-input'), exportFile());

    expect(await screen.findByTestId('import-upload-error')).toBeInTheDocument();
    expect(finishUpload).not.toHaveBeenCalled();
  });

  it('cancels an upload that waits', async () => {
    getCurrentImport.mockResolvedValue(job({ id: 'job-7', status: 'awaiting_link', imported: 0 }));
    cancelImport.mockResolvedValue(true);
    const user = userEvent.setup();
    renderImport('apple_health_archive');

    await user.click(await screen.findByTestId('import-upload-cancel'));

    await waitFor(() => expect(cancelImport).toHaveBeenCalledWith('apple_health_archive', 'job-7'));
  });

  it('keeps sending when opening the upload moves a finished import to one that waits', async () => {
    getCurrentImport
      .mockResolvedValueOnce(job({ id: 'job-1', status: 'completed' }))
      .mockResolvedValue(job({ id: 'job-9', status: 'awaiting_link', imported: 0 }));
    startUpload.mockResolvedValue({ import: job({ id: 'job-9', status: 'awaiting_link' }), upload: SESSION });
    finishUpload.mockResolvedValue(job({ id: 'job-9', status: 'downloading' }));
    // Answers only once the screen has re-read the import, so an unmount in between would abort this request.
    let reread = false;
    const fetchMock = vi.fn(async (_url: string, init: RequestInit) => {
      await waitFor(() => expect(getCurrentImport.mock.calls.length).toBeGreaterThan(1));
      await waitFor(() => expect(screen.getByTestId('import-upload-cancel')).toBeInTheDocument());
      reread = true;
      if (init.signal?.aborted) throw new DOMException('aborted', 'AbortError');
      const range = (init.headers as Record<string, string>)['Content-Range'];
      return { status: CHUNK_RANGE.test(range) ? 201 : 308, headers: { get: () => null } } as unknown as Response;
    });
    vi.stubGlobal('fetch', fetchMock);
    const user = userEvent.setup();
    renderImport('apple_health_archive');

    await user.upload(await screen.findByTestId('import-upload-pick-input'), exportFile());

    await waitFor(() => expect(finishUpload).toHaveBeenCalledWith('apple_health_archive', 'job-9'));
    expect(reread).toBe(true);
  });

  it('offers another file after a finished import, with no link to wait on', async () => {
    getCurrentImport.mockResolvedValue(job({ source: 'apple_health_archive', status: 'completed' }));
    renderImport('apple_health_archive');

    expect(await screen.findByTestId('import-upload-pick')).toBeInTheDocument();
    expect(screen.queryByText('providers.import.strava.importNewer')).not.toBeInTheDocument();
  });
});
