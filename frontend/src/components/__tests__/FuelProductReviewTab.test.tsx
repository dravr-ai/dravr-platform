// ABOUTME: Tests for the super-admin fuel-product review tab
// ABOUTME: Mocks the admin API; asserts the paged queue and catalogue, every photo and the reader's notes, identity and fact edits, the gate's refusal and reject
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
    getFuelProposalImage: vi.fn(),
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
    image_count: 2,
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
    notes: 'Energy read from the panel; the front prints 110 kcal',
    ingredients_text: 'Maltodextrin, fructose, water',
    multiple_transportable_stated: true,
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

const IMAGES = [
  { position: 0, mime: 'image/jpeg', byte_len: 2048, sha256: 'a'.repeat(64) },
  { position: 1, mime: 'image/png', byte_len: 4096, sha256: 'b'.repeat(64) },
];

function catalogue(): FuelCatalogueListResponse {
  return {
    limit: 50,
    offset: 0,
    total: 1,
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
      total: 1,
      limit: 50,
      offset: 0,
    });
    vi.mocked(adminApi.getFuelProposal).mockResolvedValue({ proposal: proposal(), images: IMAGES });
    vi.mocked(adminApi.getFuelProposalImage).mockResolvedValue(IMAGE);
    vi.mocked(adminApi.listFuelCatalogue).mockResolvedValue(catalogue());
  });

  it('lists pending proposals and re-queries when the status filter changes', async () => {
    renderTab();
    const list = await screen.findByTestId('fuel-proposals-list');
    expect(within(list).getByText('Maurten · Gel 100')).toBeInTheDocument();
    expect(within(list).getByText('1 flag')).toBeInTheDocument();
    expect(adminApi.listFuelProposals).toHaveBeenCalledWith('pending', 50, 0);
    // One page holds the whole queue: no page controls.
    expect(screen.queryByTestId('fuel-proposals-pager')).toBeNull();

    fireEvent.change(screen.getByLabelText('Proposal status'), { target: { value: 'rejected' } });
    await waitFor(() => expect(adminApi.listFuelProposals).toHaveBeenCalledWith('rejected', 50, 0));
  });

  it('pages through a queue longer than one page by its total', async () => {
    vi.mocked(adminApi.listFuelProposals).mockImplementation(async (status, _limit, offset = 0) => ({
      proposals: [proposal({ id: `${PROPOSAL_ID.slice(0, -3)}${String(offset).padStart(3, '0')}` })],
      status,
      total: 120,
      limit: 50,
      offset,
    }));
    renderTab();
    const pager = await screen.findByTestId('fuel-proposals-pager');
    expect(pager).toHaveTextContent('1–1 of 120 pending proposals');
    expect(within(pager).getByRole('button', { name: 'Previous' })).toBeDisabled();

    fireEvent.click(within(pager).getByRole('button', { name: 'Next' }));
    await waitFor(() => expect(adminApi.listFuelProposals).toHaveBeenCalledWith('pending', 50, 50));
    expect(await screen.findByText('51–51 of 120 pending proposals')).toBeInTheDocument();

    // A new status starts again from its first page.
    fireEvent.change(screen.getByLabelText('Proposal status'), { target: { value: 'approved' } });
    await waitFor(() => expect(adminApi.listFuelProposals).toHaveBeenCalledWith('approved', 50, 0));
  });

  it('opens a drawer with every stored photo, the reader notes, the source link and each fact provenance', async () => {
    const drawer = await openDrawer();
    await waitFor(() => expect(within(drawer).getAllByTestId('fuel-proposal-image')).toHaveLength(2));
    const images = within(drawer).getAllByTestId('fuel-proposal-image');
    expect(images[0]).toHaveAttribute('src', IMAGE);
    expect(adminApi.getFuelProposalImage).toHaveBeenCalledWith(PROPOSAL_ID, 0);
    expect(adminApi.getFuelProposalImage).toHaveBeenCalledWith(PROPOSAL_ID, 1);
    expect(within(drawer).getByTestId('fuel-proposal-photo-1')).toHaveTextContent('Photo 2 of 2 · image/png · 4 KB');
    expect(within(drawer).getByTestId('fuel-proposal-notes')).toHaveTextContent('the front prints 110 kcal');
    expect(within(drawer).getByTestId('fuel-proposal-ingredients')).toHaveTextContent('Maltodextrin, fructose, water');
    expect(within(drawer).getByTestId('fuel-proposal-blend-claim')).toHaveTextContent(
      'the pack claims a glucose + fructose blend',
    );
    expect(within(drawer).getByTestId('fuel-proposal-source-link')).toHaveAttribute(
      'href',
      'https://shop.example/products/gel-100',
    );
    expect(within(drawer).getByTestId('fuel-proposal-gate-flags')).toHaveTextContent('ratio_out_of_range');
    expect(within(drawer).getByTestId('fuel-fact-carbs_g')).toHaveTextContent('From label OCR, confidence 93%');
    expect(within(drawer).getByTestId('fuel-fact-sodium_mg')).toHaveTextContent('No recorded source');
  });

  it('says when no photo is stored and nobody read the pack', async () => {
    vi.mocked(adminApi.getFuelProposal).mockResolvedValue({
      proposal: proposal({
        image_count: 0,
        notes: null,
        ingredients_text: null,
        multiple_transportable_stated: null,
      }),
      images: [],
    });
    const drawer = await openDrawer();
    expect(await within(drawer).findByTestId('fuel-proposal-no-photos')).toBeInTheDocument();
    expect(within(drawer).getByTestId('fuel-proposal-blend-claim')).toHaveTextContent('not read');
    expect(adminApi.getFuelProposalImage).not.toHaveBeenCalled();
  });

  it('never renders a non-web source URL as a link', async () => {
    vi.mocked(adminApi.getFuelProposal).mockResolvedValue({
      proposal: proposal({ source_url: 'javascript:alert(1)' }),
      images: IMAGES,
    });
    const drawer = await openDrawer();
    expect(await within(drawer).findByTestId('fuel-proposal-source-text')).toHaveTextContent('javascript:alert(1)');
    expect(within(drawer).queryByTestId('fuel-proposal-source-link')).toBeNull();
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

  it('sends identity corrections with the facts, and refuses a blank name before sending', async () => {
    vi.mocked(adminApi.approveFuelProposal).mockResolvedValue({
      outcome: 'approved',
      product: catalogue().products[0],
    });
    const drawer = await openDrawer();
    const name = await within(drawer).findByLabelText('Name');
    fireEvent.change(name, { target: { value: '   ' } });
    expect(within(drawer).getByText('Name needs a value.')).toBeInTheDocument();
    expect(within(drawer).getByRole('button', { name: /Save and approve/ })).toBeDisabled();

    fireEvent.change(name, { target: { value: ' Gel 100 Caf ' } });
    fireEvent.change(within(drawer).getByLabelText('Variant'), { target: { value: 'Caf 100' } });
    fireEvent.change(within(drawer).getByLabelText('Form'), { target: { value: 'puree' } });
    // Restating the barcode with spaces is not an edit; clearing it is.
    fireEvent.change(within(drawer).getByLabelText('Barcode (UPC, EAN or GTIN-14)'), {
      target: { value: '0735 0053 6300 50' },
    });
    expect(within(drawer).getByRole('button', { name: /Save and approve \(3 edits\)/ })).toBeEnabled();
    fireEvent.click(within(drawer).getByRole('button', { name: 'Clear Barcode' }));
    fireEvent.click(within(drawer).getByRole('button', { name: /Save and approve \(4 edits\)/ }));

    await waitFor(() =>
      expect(adminApi.approveFuelProposal).toHaveBeenCalledWith(PROPOSAL_ID, {
        name: 'Gel 100 Caf',
        variant: 'Caf 100',
        form: 'puree',
        gtin: null,
      }),
    );
  });

  it('explains an identity conflict without closing the drawer', async () => {
    vi.mocked(adminApi.approveFuelProposal).mockResolvedValue({
      outcome: 'identity_conflict',
      error: 'conflict',
      product_id: 'p-1',
    });
    const drawer = await openDrawer();
    fireEvent.change(await within(drawer).findByLabelText('Brand'), { target: { value: 'SiS' } });
    fireEvent.click(within(drawer).getByRole('button', { name: /Save and approve \(1 edit\)/ }));
    expect(await within(drawer).findByTestId('fuel-review-problem')).toHaveTextContent(
      'never renames a live product',
    );
    expect(adminApi.approveFuelProposal).toHaveBeenCalledWith(PROPOSAL_ID, { brand: 'SiS' });
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
    expect(adminApi.listFuelCatalogue).toHaveBeenCalledWith(50, 0);
    expect(screen.queryByTestId('fuel-catalogue-pager')).toBeNull();
  });

  it('pages the live catalogue by its total', async () => {
    // 75 products: a full first page of 50, then the last 25.
    const row = catalogue().products[0];
    vi.mocked(adminApi.listFuelCatalogue).mockImplementation(async (_limit, offset = 0) => ({
      products: Array.from({ length: offset === 0 ? 50 : 25 }, (_, i) => ({
        ...row,
        product: { ...row.product, id: `p-${offset + i}` },
      })),
      limit: 50,
      offset,
      total: 75,
    }));
    renderTab();
    fireEvent.click(await screen.findByRole('tab', { name: 'Live catalogue' }));
    const pager = await screen.findByTestId('fuel-catalogue-pager');
    expect(pager).toHaveTextContent('1–50 of 75 products');
    fireEvent.click(within(pager).getByRole('button', { name: 'Next' }));
    await waitFor(() => expect(adminApi.listFuelCatalogue).toHaveBeenCalledWith(50, 50));
    expect(await screen.findByText('51–75 of 75 products')).toBeInTheDocument();
    const second = screen.getByTestId('fuel-catalogue-pager');
    expect(within(second).getByRole('button', { name: 'Next' })).toBeDisabled();
    expect(within(second).getByRole('button', { name: 'Previous' })).toBeEnabled();
  });
});
