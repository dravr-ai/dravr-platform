// ABOUTME: Tests for the super-admin fuel-product review tab
// ABOUTME: Mocks the admin API; asserts the queue, the drawer's image and provenance, set/clear overrides, the gate's refusal, reject and the live catalogue
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, within } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import FuelProductReviewTab from '../FuelProductReviewTab';
import type { FuelProductProposal, FuelCatalogueListResponse } from '../../services/api/admin';

vi.mock('../../services/api/admin', async () => ({
  adminApi: {
    listFuelProposals: vi.fn(),
    getFuelProposal: vi.fn(),
    getFuelProposalSourceImage: vi.fn(),
    approveFuelProposal: vi.fn(),
    rejectFuelProposal: vi.fn(),
    listFuelCatalogue: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api/admin');

const PROPOSAL_ID = '6f1c2b8e-0000-4000-8000-000000000001';
const IMAGE = 'data:image/png;base64,iVBORw0KGgo=';

function proposal(overrides: Partial<FuelProductProposal> = {}): FuelProductProposal {
  return {
    id: PROPOSAL_ID,
    product_id: null,
    brand: 'Maurten',
    name: 'Gel 100',
    variant: '',
    form: 'gel',
    gtin: '07350053630050',
    source_kind: 'shopify_ocr',
    source_url: 'https://shop.example/products/gel-100',
    has_source_image: true,
    source_image_mime: 'image/png',
    source_hash: 'abc',
    facts: {
      serving_label: '1 gel (40 g)',
      serving_g: 40,
      energy_kcal: 100,
      carbs_g: 25,
      sugars_g: 12,
      glucose_fructose_ratio: 0.8,
    },
    provenance: {
      carbs_g: { source: 'label_ocr', confidence: 0.93 },
      glucose_fructose_ratio: { source: 'label_ocr', confidence: 0.4 },
    },
    gate_flags: ['ratio_out_of_range'],
    status: 'pending',
    proposed_by_user_id: null,
    reviewed_by_user_id: null,
    reviewed_at: null,
    reject_reason: null,
    reject_note: null,
    approved_version_id: null,
    created_at: '2026-10-29T12:00:00Z',
    ...overrides,
  };
}

function catalogue(): FuelCatalogueListResponse {
  return {
    limit: 100,
    offset: 0,
    products: [
      {
        product: {
          id: 'p-1',
          gtin: null,
          brand: 'SiS',
          name: 'Beta Fuel',
          variant: 'Orange',
          form: 'drink_mix',
          created_at: '2026-10-01T00:00:00Z',
        },
        current: {
          id: 'v-2',
          product_id: 'p-1',
          valid_from: '2026-10-20T00:00:00Z',
          superseded_by: null,
          facts: { carbs_g: 80, glucose_fructose_ratio: 0.8 },
          provenance: {},
          approved_by_user_id: 'u-1',
          approved_at: '2026-10-20T00:00:00Z',
          created_at: '2026-10-20T00:00:00Z',
        },
        version_count: 2,
      },
    ],
  };
}

function renderTab() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <FuelProductReviewTab />
    </QueryClientProvider>,
  );
}

async function openDrawer() {
  renderTab();
  fireEvent.click(await screen.findByTestId(`fuel-proposal-open-${PROPOSAL_ID}`));
  return await screen.findByTestId('fuel-proposal-drawer');
}

describe('FuelProductReviewTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(adminApi.listFuelProposals).mockResolvedValue({
      proposals: [proposal()],
      status: 'pending',
      limit: 100,
      offset: 0,
    });
    vi.mocked(adminApi.getFuelProposal).mockResolvedValue({
      proposal: proposal(),
      source_image_bytes: 1234,
    });
    vi.mocked(adminApi.getFuelProposalSourceImage).mockResolvedValue(IMAGE);
    vi.mocked(adminApi.listFuelCatalogue).mockResolvedValue(catalogue());
  });

  it('lists pending proposals and re-queries when the status filter changes', async () => {
    renderTab();
    const list = await screen.findByTestId('fuel-proposals-list');
    expect(within(list).getByText('Maurten · Gel 100')).toBeInTheDocument();
    expect(within(list).getByText('1 flag')).toBeInTheDocument();
    expect(adminApi.listFuelProposals).toHaveBeenCalledWith('pending', 100);

    fireEvent.change(screen.getByLabelText('Proposal status'), { target: { value: 'rejected' } });
    await waitFor(() => expect(adminApi.listFuelProposals).toHaveBeenCalledWith('rejected', 100));
  });

  it('opens a drawer with the served label image, the source link and each fact provenance', async () => {
    const drawer = await openDrawer();
    const image = await within(drawer).findByTestId('fuel-proposal-image');
    expect(image).toHaveAttribute('src', IMAGE);
    expect(adminApi.getFuelProposalSourceImage).toHaveBeenCalledWith(PROPOSAL_ID);
    expect(within(drawer).getByTestId('fuel-proposal-source-link')).toHaveAttribute(
      'href',
      'https://shop.example/products/gel-100',
    );
    expect(within(drawer).getByTestId('fuel-proposal-gate-flags')).toHaveTextContent('ratio_out_of_range');
    expect(within(drawer).getByTestId('fuel-fact-carbs_g')).toHaveTextContent('From label OCR, confidence 93%');
    expect(within(drawer).getByTestId('fuel-fact-sodium_mg')).toHaveTextContent('No recorded source');
  });

  it('sends a changed value and a cleared one with Save and approve', async () => {
    vi.mocked(adminApi.approveFuelProposal).mockResolvedValue({
      outcome: 'approved',
      product: catalogue().products[0],
    });
    const drawer = await openDrawer();
    fireEvent.change(await within(drawer).findByLabelText('Energy (kcal)'), { target: { value: '110' } });
    // Restating the carbs as printed is not an edit.
    fireEvent.change(within(drawer).getByLabelText('Carbohydrate (g)'), { target: { value: '25' } });
    fireEvent.click(within(drawer).getByRole('button', { name: 'Clear Fructose per gram of glucose' }));

    const approveAsProposed = within(drawer).getByRole('button', { name: 'Approve as proposed' });
    expect(approveAsProposed).toBeDisabled();
    fireEvent.click(within(drawer).getByRole('button', { name: /Save and approve \(2 edits\)/ }));

    await waitFor(() =>
      expect(adminApi.approveFuelProposal).toHaveBeenCalledWith(PROPOSAL_ID, {
        energy_kcal: 110,
        glucose_fructose_ratio: null,
      }),
    );
    expect(await screen.findByTestId('fuel-review-notice')).toHaveTextContent('Approved: SiS · Beta Fuel · Orange is live.');
    expect(screen.queryByTestId('fuel-proposal-drawer')).not.toBeInTheDocument();
  });

  it('shows the gate violations when the merged facts are implausible', async () => {
    vi.mocked(adminApi.approveFuelProposal).mockResolvedValue({
      outcome: 'implausible',
      error: 'The facts fail the plausibility gate; nothing was written',
      violations: [{ code: 'sugars_exceed_carbs', message: 'sugars 30 g exceed carbs 25 g (per serving)' }],
    });
    const drawer = await openDrawer();
    fireEvent.click(await within(drawer).findByRole('button', { name: 'Approve as proposed' }));

    await waitFor(() => expect(adminApi.approveFuelProposal).toHaveBeenCalledWith(PROPOSAL_ID, {}));
    const problem = await within(drawer).findByTestId('fuel-review-problem');
    expect(problem).toHaveTextContent('sugars_exceed_carbs');
    expect(screen.getByTestId('fuel-proposal-drawer')).toBeInTheDocument();
  });

  it('says why a proposal that was already reviewed cannot be approved', async () => {
    vi.mocked(adminApi.approveFuelProposal).mockResolvedValue({
      outcome: 'not_pending',
      error: 'The proposal was already rejected',
      status: 'rejected',
    });
    const drawer = await openDrawer();
    fireEvent.click(await within(drawer).findByRole('button', { name: 'Approve as proposed' }));
    expect(await within(drawer).findByTestId('fuel-review-problem')).toHaveTextContent(
      'Already reviewed: this proposal is rejected.',
    );
  });

  it('rejects with a reason and a note after confirming', async () => {
    vi.mocked(adminApi.rejectFuelProposal).mockResolvedValue({
      outcome: 'rejected',
      proposal: proposal({ status: 'rejected', reject_reason: 'wrong_product' }),
    });
    const drawer = await openDrawer();
    const reject = await within(drawer).findByRole('button', { name: 'Reject' });
    expect(reject).toBeDisabled();

    fireEvent.change(within(drawer).getByLabelText('Reason'), { target: { value: 'other' } });
    expect(reject).toBeDisabled();
    fireEvent.change(within(drawer).getByLabelText('Note'), { target: { value: 'Panel is from the bar' } });
    fireEvent.click(reject);

    // ConfirmDialog renders its title in the body, so the dialog has no name.
    const title = await screen.findByText('Reject this proposal?');
    const confirm = title.closest('[role="dialog"]') as HTMLElement;
    expect(adminApi.rejectFuelProposal).not.toHaveBeenCalled();
    fireEvent.click(within(confirm).getByRole('button', { name: 'Reject' }));
    await waitFor(() =>
      expect(adminApi.rejectFuelProposal).toHaveBeenCalledWith(PROPOSAL_ID, 'other', 'Panel is from the bar'),
    );
    expect(await screen.findByTestId('fuel-review-notice')).toHaveTextContent('Rejected Maurten · Gel 100.');
  });

  it('lists the live catalogue with each product version count', async () => {
    renderTab();
    fireEvent.click(await screen.findByRole('tab', { name: 'Live catalogue' }));
    const list = await screen.findByTestId('fuel-catalogue-list');
    expect(list).toHaveTextContent('SiS · Beta Fuel · Orange');
    expect(list).toHaveTextContent('80 g carbs per serving');
    expect(list).toHaveTextContent('2 versions');
  });
});
