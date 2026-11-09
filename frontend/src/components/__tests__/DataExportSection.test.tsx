// ABOUTME: Unit tests for the web data export: the button fetches through userApi and hands the browser a JSON file
// ABOUTME: A refused or cut export shows an alert and saves nothing; a partial one is saved and says which limit stopped it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import { EXPORT_PARTIAL_KEYS } from '@pierre/ui-logic';
import DataExportSection from '../DataExportSection';

const exportMyData = vi.fn();

/** Rows the way the server writes them, without the document's end. */
const ROWS = '{"format":"dravr-user-export","tables":{"users":[{"id":"a"}';
/** A whole, complete document. */
const WHOLE = `${ROWS}]},"complete":true,"limit_reached":null,"failed_tables":[],"skipped_tables":[]}`;

vi.mock('../../services/api', () => ({
  dataExportApi: { exportMyData: () => exportMyData() },
}));

function renderSection() {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <DataExportSection />
    </QueryClientProvider>,
  );
}

/** jsdom's Blob has no text(); read it the way a browser page would. */
function readText(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsText(blob);
  });
}

describe('DataExportSection', () => {
  const createObjectURL = vi.fn(() => 'blob:export');
  const revokeObjectURL = vi.fn();
  let clicked: HTMLAnchorElement[];

  beforeEach(async () => {
    vi.clearAllMocks();
    await i18n.changeLanguage('en');
    clicked = [];
    Object.assign(URL, { createObjectURL, revokeObjectURL });
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (
      this: HTMLAnchorElement,
    ) {
      clicked.push(this);
    });
  });

  it('downloads the document the server sent as a dated .json file', async () => {
    exportMyData.mockResolvedValue({
      fileName: 'dravr-export-2026-11-04.json',
      content: new Blob([WHOLE]),
    });
    renderSection();

    fireEvent.click(screen.getByTestId('data-export-action'));

    await waitFor(() => expect(clicked).toHaveLength(1));
    expect(clicked[0].download).toBe('dravr-export-2026-11-04.json');
    const blob = (createObjectURL.mock.calls[0] as unknown as [Blob])[0];
    expect(blob.type).toBe('application/json');
    expect(await readText(blob)).toBe(WHOLE);
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:export');
    expect(screen.queryByTestId('data-export-failure')).toBeNull();
    expect(screen.queryByTestId('data-export-partial')).toBeNull();
  });

  // The notice is t(key). Each key's text is its own, so the text names the
  // key: a platform copy that lacks a key renders the key itself.
  it('says each partial file in its own words', () => {
    const texts = EXPORT_PARTIAL_KEYS.map((key) => i18n.t(key));
    expect(new Set(texts).size).toBe(EXPORT_PARTIAL_KEYS.length);
  });

  it.each([
    ['its size limit', '"limit_reached":"size",', '[]', 'dataExport.partialSize'],
    ['its time limit', '"limit_reached":"time",', '[]', 'dataExport.partialTime'],
    [
      'its size limit with an unread table',
      '"limit_reached":"size",',
      '["recipes"]',
      'dataExport.partialSizeAndFailed',
    ],
    ['a limit it does not name', '', '[]', 'dataExport.partialLimit'],
    [
      'a limit it does not name with an unread table',
      '',
      '["recipes"]',
      'dataExport.partialLimitAndFailed',
    ],
  ])('saves a file the server stopped at %s, and says so', async (_case, limit, failed, key) => {
    exportMyData.mockResolvedValue({
      fileName: 'dravr-export-2026-11-04.json',
      content: new Blob([
        ROWS,
        `]},"complete":false,${limit}"failed_tables":${failed},"skipped_tables":["activity_streams"]}`,
      ]),
    });
    renderSection();

    fireEvent.click(screen.getByTestId('data-export-action'));

    const notice = await screen.findByTestId('data-export-partial');
    expect(notice).toHaveAttribute('role', 'status');
    // Exact: one key's text can hold another's (partialSize in partialSizeAndFailed).
    expect(notice.textContent).toBe(i18n.t(key));
    expect(clicked).toHaveLength(1);
    expect(screen.queryByTestId('data-export-failure')).toBeNull();
  });

  it('saves a file with a table it could not read, and says so', async () => {
    exportMyData.mockResolvedValue({
      fileName: 'dravr-export-2026-11-04.json',
      content: new Blob([
        ROWS,
        ']},"complete":false,"limit_reached":null,"failed_tables":["recipes"],"skipped_tables":[]}',
      ]),
    });
    renderSection();

    fireEvent.click(screen.getByTestId('data-export-action'));

    const notice = await screen.findByTestId('data-export-partial');
    expect(notice.textContent).toBe(i18n.t('dataExport.partial'));
  });

  it('saves nothing when the download stops inside the document', async () => {
    exportMyData.mockResolvedValue({
      fileName: 'dravr-export-2026-11-04.json',
      content: new Blob([ROWS]),
    });
    renderSection();

    fireEvent.click(screen.getByTestId('data-export-action'));

    expect(await screen.findByTestId('data-export-failure')).toHaveAttribute('role', 'alert');
    expect(clicked).toHaveLength(0);
  });

  it('shows an alert and saves nothing when the export is refused', async () => {
    exportMyData.mockRejectedValue({ response: { status: 429 } });
    renderSection();

    fireEvent.click(screen.getByTestId('data-export-action'));

    expect(await screen.findByTestId('data-export-failure')).toHaveAttribute('role', 'alert');
    expect(clicked).toHaveLength(0);
    expect(createObjectURL).not.toHaveBeenCalled();
  });
});
