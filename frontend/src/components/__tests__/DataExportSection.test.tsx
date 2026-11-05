// ABOUTME: Unit tests for the web data export: the button fetches through userApi and hands the browser a JSON file
// ABOUTME: A refused or failed export shows an alert and saves nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import DataExportSection from '../DataExportSection';

const exportMyData = vi.fn();

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
      content: new Blob(['{"format":"dravr-user-export"}']),
    });
    renderSection();

    fireEvent.click(screen.getByTestId('data-export-action'));

    await waitFor(() => expect(clicked).toHaveLength(1));
    expect(clicked[0].download).toBe('dravr-export-2026-11-04.json');
    const blob = (createObjectURL.mock.calls[0] as unknown as [Blob])[0];
    expect(blob.type).toBe('application/json');
    expect(await readText(blob)).toBe('{"format":"dravr-user-export"}');
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:export');
    expect(screen.queryByTestId('data-export-failure')).toBeNull();
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
