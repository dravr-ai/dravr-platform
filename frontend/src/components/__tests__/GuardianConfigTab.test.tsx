// ABOUTME: Tests for the GuardianConfigTab admin form
// ABOUTME: Mocks the admin API and asserts hydration, unsaved edits, the two 409s, validation, tenant typing, history and parked calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor, act, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import GuardianConfigTab from '../GuardianConfigTab';
import type { GuardianConfigResponse } from '../../services/api/admin';

vi.mock('../../services/api/admin', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../services/api/admin')>()),
  adminApi: {
    getGuardianConfig: vi.fn(),
    putGuardianConfig: vi.fn(),
    getAuditEvents: vi.fn(),
    listGuardianPendingActions: vi.fn(),
    denyGuardianPendingAction: vi.fn(),
  },
}));

const { adminApi } = await import('../../services/api/admin');

const STAMP = '2026-08-11T12:00:00.123456+00:00';
const TENANT_A = '11111111-1111-4111-8111-111111111111';
const TENANT_B = '22222222-2222-4222-8222-222222222222';

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
    defaults: {
      mode: 'enforce',
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
    updated_at: STAMP,
    loaded_at: '2026-08-11T12:00:30+00:00',
    instance_id: 'rev-7:abc',
    in_sync: true,
    degraded: false,
    degraded_reason: null,
    plan_mode_armable: false,
    ...overrides,
  };
}

function conflict(code: string, message: string, impacted_fields?: unknown[]) {
  return { response: { status: 409, data: { code, message, impacted_fields } } };
}

const STALE = conflict(
  'guardian_config_stale',
  'The Guardian policy changed since you read it, and nothing was saved.',
);
const IMPACT = conflict('guardian_config_impact_not_acknowledged', 'Read each impacted field.', [
  { field: 'mode', from: 'enforce', to: 'observe', effect: 'The Guardian stops blocking anything.' },
  { field: 'external_send', from: 'none', to: 'all', effect: 'Changes which tenants may send.' },
]);

async function renderTab() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = render(
    <QueryClientProvider client={queryClient}>
      <GuardianConfigTab />
    </QueryClientProvider>,
  );
  await screen.findByText('Guardian security policy');
  return { queryClient, ...view };
}

const combobox = (index: number) => screen.getAllByRole('combobox')[index] as HTMLSelectElement;
const saveButton = () => screen.getByRole('button', { name: /save changes/i });

/** A refetch's answer arriving while the page is open. */
async function backgroundRefetch(queryClient: QueryClient, response: GuardianConfigResponse) {
  await act(async () => {
    queryClient.setQueryData(['admin', 'guardian-config'], response);
  });
}

describe('GuardianConfigTab', () => {
  beforeEach(() => {
    vi.resetAllMocks();
    vi.mocked(adminApi.getAuditEvents).mockResolvedValue({ events: [], next_before: null });
    vi.mocked(adminApi.listGuardianPendingActions).mockResolvedValue({
      entries: [],
      status: 'pending',
      limit: 50,
    });
  });

  it('hydrates the mode select from the persisted document', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    await renderTab();
    // Field order: mode, tainted_destructive, plan_mode, external_send.
    expect(combobox(0).value).toBe('observe');
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
    await renderTab();
    expect(combobox(0)).toBeDisabled();
    const notice = screen.getByText(/locked by GUARDIAN_\* environment variables/i);
    expect(notice).toBeInTheDocument();
    // Readable on the warning tint, in the ink the palette pairs with it.
    expect(notice.className).toContain('bg-warning/15');
    expect(notice.className).toContain('text-on-warning-container');
    // Exact string targets the field badge; the notice paragraph above has
    // longer text and is asserted separately.
    expect(screen.getByText('env-pinned')).toBeInTheDocument();
  });

  it('rejects a zero write budget before save', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    await renderTab();
    const inputs = screen.getAllByRole('spinbutton');
    // Budget order: max_destructive_per_turn, max_writes_per_turn.
    fireEvent.change(inputs[1], { target: { value: '0' } });
    expect(
      screen.getByText(/must be >= 1 \(0 would deny every write-tool dispatch\)/i),
    ).toBeInTheDocument();
    expect(saveButton()).toBeDisabled();
  });

  it.each([
    [0, 'max_destructive_per_turn', '1.5', /whole number from 0 to 65535/],
    [0, 'max_destructive_per_turn', '-1', /whole number from 0 to 65535/],
    [1, 'max_writes_per_turn', '2.5', /whole number from 1 to 65535/],
    [1, 'max_writes_per_turn', '70000', /whole number from 1 to 65535/],
  ])('names the field when input %i holds %s %s', async (input, field, value, message) => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    await renderTab();
    fireEvent.change(screen.getAllByRole('spinbutton')[input], { target: { value } });

    const problem = screen.getByText(message).closest('li');
    expect(problem).toHaveTextContent(field);
    expect(saveButton()).toBeDisabled();
    expect(adminApi.putGuardianConfig).not.toHaveBeenCalled();
  });

  it('accepts a zero destructive budget and the top of the range', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    await renderTab();
    const inputs = screen.getAllByRole('spinbutton');
    fireEvent.change(inputs[0], { target: { value: '0' } });
    fireEvent.change(inputs[1], { target: { value: '65535' } });
    expect(saveButton()).not.toBeDisabled();
  });

  it('shows a malformed-body refusal as the server worded it', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce({
      response: {
        status: 422,
        data: {
          code: 'guardian_config_malformed',
          message: 'The Guardian policy was not saved: expected u16',
        },
      },
    });
    await renderTab();
    fireEvent.click(saveButton());
    expect(await screen.findByText(/was not saved: expected u16/)).toBeInTheDocument();
  });

  it('saves the edited document with the token it started from', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(
      sampleResponse({
        config: { schema_version: 1, mode: 'enforce', tainted_destructive: 'confirm' },
      }),
    );
    await renderTab();
    fireEvent.change(combobox(1), { target: { value: 'confirm' } });
    fireEvent.click(saveButton());
    await waitFor(() => {
      expect(adminApi.putGuardianConfig).toHaveBeenCalled();
    });
    const [payload, expectedUpdatedAt, acknowledge] = vi.mocked(adminApi.putGuardianConfig).mock
      .calls[0];
    expect(payload.tainted_destructive).toBe('confirm');
    expect(payload.mode).toBe('observe');
    expect(payload.schema_version).toBe(1);
    expect(expectedUpdatedAt).toBe(STAMP);
    expect(acknowledge).toBe(false);
    // The reply re-seeds the form from what the server stored.
    await waitFor(() => expect(combobox(0).value).toBe('enforce'));
  });

  it('sends a null token when no row was read', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
      sampleResponse({ source: 'default', updated_at: null, config: { schema_version: 1 } }),
    );
    vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
    await renderTab();
    fireEvent.change(screen.getAllByRole('spinbutton')[1], { target: { value: '12' } });
    fireEvent.click(saveButton());
    await waitFor(() => expect(adminApi.putGuardianConfig).toHaveBeenCalled());
    expect(vi.mocked(adminApi.putGuardianConfig).mock.calls[0][1]).toBeNull();
  });

  it('clearing a select back to follow-default sends null for the field', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
    await renderTab();
    fireEvent.change(combobox(0), { target: { value: '' } });
    fireEvent.click(saveButton());
    await waitFor(() => {
      expect(adminApi.putGuardianConfig).toHaveBeenCalled();
    });
    const [payload] = vi.mocked(adminApi.putGuardianConfig).mock.calls[0];
    expect(payload.mode).toBeNull();
  });

  describe('unsaved edits', () => {
    const newer = sampleResponse({
      config: { schema_version: 1, mode: 'enforce' },
      updated_at: '2026-08-11T13:00:00.654321+00:00',
    });

    it('a background refetch adopts the row while nothing is edited', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      const { queryClient } = await renderTab();
      await backgroundRefetch(queryClient, newer);
      await waitFor(() => expect(combobox(0).value).toBe('enforce'));
    });

    it('a background refetch leaves edits and their token alone, and says so', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce(STALE);
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(newer);
      const { queryClient } = await renderTab();

      fireEvent.change(combobox(1), { target: { value: 'deny' } });
      await backgroundRefetch(queryClient, newer);

      expect(await screen.findByText(/stored policy changed while you were editing/i)).toBeInTheDocument();
      expect(combobox(1).value).toBe('deny');
      expect(combobox(0).value).toBe('observe');

      fireEvent.click(saveButton());
      await waitFor(() => expect(adminApi.putGuardianConfig).toHaveBeenCalled());
      expect(vi.mocked(adminApi.putGuardianConfig).mock.calls[0][1]).toBe(STAMP);
    });

    it('undoing an edit makes the form follow the row again', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      const { queryClient } = await renderTab();
      fireEvent.change(combobox(1), { target: { value: 'deny' } });
      await backgroundRefetch(queryClient, newer);
      await screen.findByText(/stored policy changed while you were editing/i);
      expect(combobox(0).value).toBe('observe');

      fireEvent.change(combobox(1), { target: { value: '' } });
      await waitFor(() => expect(combobox(0).value).toBe('enforce'));
    });

    it('a failed refetch keeps the form on screen', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.getGuardianConfig).mockRejectedValueOnce(new Error('offline'));
      const { queryClient } = await renderTab();
      fireEvent.change(combobox(1), { target: { value: 'deny' } });
      await act(async () => {
        await queryClient.refetchQueries({ queryKey: ['admin', 'guardian-config'] });
      });
      await waitFor(() =>
        expect(queryClient.getQueryState(['admin', 'guardian-config'])?.status).toBe('error'),
      );
      expect(screen.queryByText(/Failed to load guardian config/)).toBeNull();
      expect(combobox(1).value).toBe('deny');
    });
  });

  it('a stale save reloads the form and names what it discarded', async () => {
    const stored = sampleResponse({
      config: { schema_version: 1, mode: 'enforce' },
      updated_at: '2026-08-11T13:00:00.654321+00:00',
    });
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce(STALE);
    vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(stored);
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(stored);
    await renderTab();

    fireEvent.change(combobox(1), { target: { value: 'deny' } });
    fireEvent.click(saveButton());

    const notice = await screen.findByText(/changed since you read it/i);
    expect(notice).toHaveTextContent('nothing was saved');
    expect(notice).toHaveTextContent('Your edits to tainted_destructive were discarded');
    await waitFor(() => expect(combobox(0).value).toBe('enforce'));
    expect(combobox(1).value).toBe('');

    // The next save starts from the row it just reloaded.
    fireEvent.click(saveButton());
    await waitFor(() => expect(adminApi.putGuardianConfig).toHaveBeenCalledTimes(2));
    expect(vi.mocked(adminApi.putGuardianConfig).mock.calls[1][1]).toBe(stored.updated_at);
  });

  it('a stale save keeps the edits when the reload fails, and says so', async () => {
    vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
    vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce(STALE);
    vi.mocked(adminApi.getGuardianConfig).mockRejectedValueOnce(new Error('unreachable'));
    await renderTab();

    fireEvent.change(combobox(1), { target: { value: 'deny' } });
    fireEvent.click(saveButton());

    const notice = await screen.findByText(/could not be reloaded/i);
    expect(notice).toHaveTextContent('your edits are still here');
    expect(combobox(1).value).toBe('deny');
  });

  describe('a high-impact save', () => {
    it('lists each field inline and sends the acknowledgement on the second click', async () => {
      const confirmSpy = vi.spyOn(window, 'confirm');
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce(IMPACT);
      vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();

      fireEvent.change(combobox(1), { target: { value: 'deny' } });
      fireEvent.click(saveButton());

      const panel = (await screen.findByText(/changes what athletes can do/i)).closest(
        '[role="alert"]',
      ) as HTMLElement;
      expect(within(panel).getByText(/The Guardian stops blocking anything\./)).toHaveTextContent(
        'mode: enforce → observe.',
      );
      expect(within(panel).getByText(/Changes which tenants may send\./)).toHaveTextContent(
        'external_send: none → all.',
      );
      expect(confirmSpy).not.toHaveBeenCalled();
      expect(adminApi.putGuardianConfig).toHaveBeenCalledTimes(1);

      fireEvent.click(within(panel).getByRole('button', { name: /confirm and save/i }));
      await waitFor(() => expect(adminApi.putGuardianConfig).toHaveBeenCalledTimes(2));
      const [first, second] = vi.mocked(adminApi.putGuardianConfig).mock.calls;
      expect(first[2]).toBe(false);
      expect(second[2]).toBe(true);
      expect(second[0]).toEqual(first[0]);
      expect(second[1]).toBe(first[1]);
      await waitFor(() => expect(screen.queryByText(/changes what athletes can do/i)).toBeNull());
    });

    it('keep editing dismisses the panel and sends nothing', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce(IMPACT);
      await renderTab();
      fireEvent.click(saveButton());
      fireEvent.click(await screen.findByRole('button', { name: /keep editing/i }));
      expect(screen.queryByText(/changes what athletes can do/i)).toBeNull();
      expect(adminApi.putGuardianConfig).toHaveBeenCalledTimes(1);
    });

    it('an edit after the refusal drops the panel, so no acknowledgement outlives its list', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.putGuardianConfig).mockRejectedValueOnce(IMPACT);
      vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      fireEvent.click(saveButton());
      await screen.findByText(/changes what athletes can do/i);

      fireEvent.change(combobox(0), { target: { value: 'off' } });
      expect(screen.queryByText(/changes what athletes can do/i)).toBeNull();

      fireEvent.click(saveButton());
      await waitFor(() => expect(adminApi.putGuardianConfig).toHaveBeenCalledTimes(2));
      expect(vi.mocked(adminApi.putGuardianConfig).mock.calls[1][2]).toBe(false);
    });
  });

  describe('what the instance reports', () => {
    it('shows not in sync, degraded with its reason, and which instance answered', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
        sampleResponse({
          in_sync: false,
          degraded: true,
          degraded_reason: 'the stored row does not parse (bad json)',
          stored_unknown_fields: ['future_field'],
          stored_schema_version: 0,
        }),
      );
      await renderTab();
      expect(screen.getByText('not in sync')).toBeInTheDocument();
      expect(screen.getByText(/does not yet enforce the stored policy/i)).toBeInTheDocument();
      expect(screen.getByText('degraded')).toBeInTheDocument();
      expect(screen.getByText(/the stored row does not parse \(bad json\)/)).toBeInTheDocument();
      expect(screen.getByText('rev-7:abc')).toBeInTheDocument();
      expect(screen.getByText(/policy\s+loaded/)).toBeInTheDocument();
      expect(screen.getByText(/a save drops them:\s+future_field/)).toBeInTheDocument();
      expect(screen.getByText(/schema version 0/)).toBeInTheDocument();
    });

    it('shows neither flag on a healthy instance', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      expect(screen.queryByText('not in sync')).toBeNull();
      expect(screen.queryByText('degraded')).toBeNull();
    });
  });

  describe('plan mode', () => {
    it('disables enforce with its reason when the deployment cannot arm it', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      const enforce = within(combobox(2)).getByRole('option', { name: 'enforce' });
      expect(enforce).toBeDisabled();
      expect(screen.getByText(/GUARDIAN_PLAN_MODE_ARMABLE is not set/)).toBeInTheDocument();
    });

    it('offers enforce when it can', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
        sampleResponse({ plan_mode_armable: true }),
      );
      await renderTab();
      expect(within(combobox(2)).getByRole('option', { name: 'enforce' })).not.toBeDisabled();
      expect(screen.queryByText(/GUARDIAN_PLAN_MODE_ARMABLE is not set/)).toBeNull();
    });

    it('blocks a save of a row already armed where it cannot be', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
        sampleResponse({ config: { schema_version: 1, plan_mode: 'enforce' } }),
      );
      await renderTab();
      expect(saveButton()).toBeDisabled();
      fireEvent.change(combobox(2), { target: { value: 'off' } });
      expect(saveButton()).not.toBeDisabled();
    });

    it('leaves a refused value fixable under an env pin, and locks the field otherwise', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
        sampleResponse({
          env_pinned: ['plan_mode'],
          config: { schema_version: 1, plan_mode: 'enforce' },
        }),
      );
      const { unmount } = await renderTab();
      expect(saveButton()).toBeDisabled();
      expect(combobox(2)).not.toBeDisabled();
      fireEvent.change(combobox(2), { target: { value: 'off' } });
      expect(saveButton()).not.toBeDisabled();
      unmount();

      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
        sampleResponse({ env_pinned: ['plan_mode'] }),
      );
      await renderTab();
      expect(combobox(2)).toBeDisabled();
    });
  });

  describe('what the text says', () => {
    it('follow default names the compiled default and in force names the effective value', async () => {
      // Observe is in force and enforce is the default: two different answers.
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      expect(within(combobox(0)).getByRole('option', { name: 'Follow default (enforce)' })).toBeInTheDocument();
      expect(screen.getByText('In force on this instance: observe')).toBeInTheDocument();
    });

    it('a budget placeholder names the compiled default, not what is in force', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(
        sampleResponse({
          effective: { ...sampleResponse().effective, max_writes_per_turn: 7 },
        }),
      );
      await renderTab();
      expect(screen.getByPlaceholderText('default: 50')).toBeInTheDocument();
      expect(screen.getByText('In force on this instance: 7')).toBeInTheDocument();
    });

    it('says off and observe behave the same, and states the propagation bound', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      expect(screen.getByText(/off and observe behave the same at runtime/)).toBeInTheDocument();
      expect(screen.queryByText(/for debugging only/)).toBeNull();
      expect(screen.getByText(/within its settings poll \(30 seconds/)).toBeInTheDocument();
      expect(screen.queryByText(/next tool dispatch/)).toBeNull();
    });
  });

  describe('the tenant list', () => {
    const tenantList = () => screen.getByLabelText(/Tenant UUIDs/i) as HTMLInputElement;

    async function openList() {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      fireEvent.change(combobox(3), { target: { value: 'tenants' } });
    }

    it('keeps a typed comma and space, and tidies them on blur', async () => {
      const user = userEvent.setup();
      await openList();

      await user.type(tenantList(), `${TENANT_A}, `);
      expect(tenantList().value).toBe(`${TENANT_A}, `);

      await user.type(tenantList(), TENANT_B);
      expect(tenantList().value).toBe(`${TENANT_A}, ${TENANT_B}`);

      await user.type(tenantList(), ' ,');
      expect(tenantList().value).toBe(`${TENANT_A}, ${TENANT_B} ,`);
      await user.tab();
      expect(tenantList().value).toBe(`${TENANT_A}, ${TENANT_B}`);
    });

    it('parses what is typed on save, without a blur', async () => {
      await openList();
      fireEvent.change(tenantList(), { target: { value: `${TENANT_A},${TENANT_B},` } });
      fireEvent.click(saveButton());
      await waitFor(() => expect(adminApi.putGuardianConfig).toHaveBeenCalled());
      expect(vi.mocked(adminApi.putGuardianConfig).mock.calls[0][0].external_send).toEqual([
        TENANT_A,
        TENANT_B,
      ]);
    });

    it('refuses an empty list and a bad id before save', async () => {
      await openList();
      expect(screen.getByText(/name at least one tenant UUID/)).toBeInTheDocument();
      expect(saveButton()).toBeDisabled();

      fireEvent.change(tenantList(), { target: { value: 'nope,' } });
      expect(screen.getByText(/"nope" is not a tenant UUID/)).toBeInTheDocument();
      expect(saveButton()).toBeDisabled();

      fireEvent.change(tenantList(), { target: { value: TENANT_A } });
      expect(saveButton()).not.toBeDisabled();
    });
  });

  describe('the history table', () => {
    const saveEvent = {
      action: 'guardian.config.update',
      summary: 'moved',
      outcome: 'success' as const,
      source: 'console' as const,
      actor_user_id: 'u',
      actor_email: 'op@dravr.ai',
      tenant_id: null,
    };

    it('reads the audit trail and says what each save moved', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.getAuditEvents).mockResolvedValue({
        events: [
          {
            ...saveEvent,
            id: 'a',
            occurred_at: '2026-08-11T13:00:00+00:00',
            metadata: {
              before: { schema_version: 1, mode: 'observe' },
              after: { schema_version: 1, mode: 'enforce', external_send: [TENANT_A] },
            },
          },
          {
            ...saveEvent,
            id: 'b',
            occurred_at: '2026-08-11T12:00:00+00:00',
            actor_email: null,
            actor_user_id: null,
            source: 'service',
            metadata: { before: '{broken', after: { schema_version: 1 } },
          },
          {
            ...saveEvent,
            id: 'c',
            occurred_at: '2026-08-11T11:00:00+00:00',
            outcome: 'refused',
            summary: 'Refused a Guardian policy save (409)',
            metadata: { status: 409 },
          },
        ],
        next_before: null,
      });
      await renderTab();
      expect(await screen.findByText('mode: observe → enforce')).toBeInTheDocument();
      expect(screen.getByText(`external_send: default → ${TENANT_A}`)).toBeInTheDocument();
      expect(screen.getAllByText('op@dravr.ai')).toHaveLength(2);
      expect(screen.getByText('service token')).toBeInTheDocument();
      expect(screen.getByText('The previous row was not a document')).toBeInTheDocument();
      expect(screen.getByText('Refused a Guardian policy save (409)')).toBeInTheDocument();
      expect(adminApi.getAuditEvents).toHaveBeenCalledWith({ action: 'guardian.config', limit: 10 });
    });

    it('says when there is nothing, and refreshes after a save', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.putGuardianConfig).mockResolvedValueOnce(sampleResponse());
      await renderTab();
      expect(await screen.findByText('No recorded saves yet.')).toBeInTheDocument();
      const reads = vi.mocked(adminApi.getAuditEvents).mock.calls.length;
      fireEvent.click(saveButton());
      await waitFor(() =>
        expect(vi.mocked(adminApi.getAuditEvents).mock.calls.length).toBeGreaterThan(reads),
      );
    });
  });

  describe('the parked calls table', () => {
    const parked = {
      id: 'call-a',
      tenant_id: TENANT_A,
      user_id: TENANT_B,
      conversation_id: null,
      tool_name: 'disconnect_provider',
      deny_reason: 'tainted_sink',
      status: 'pending' as const,
      turn_agent_id: null,
      seat: 'athlete',
      created_at: '2026-08-11T12:00:00+00:00',
      expires_at: '2026-08-11T12:05:00+00:00',
      resolved_at: null,
    };

    it('denies only on the second click, then rereads the list', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.listGuardianPendingActions)
        .mockResolvedValueOnce({ entries: [parked], status: 'pending', limit: 50 })
        .mockResolvedValue({ entries: [], status: 'pending', limit: 50 });
      vi.mocked(adminApi.denyGuardianPendingAction).mockResolvedValueOnce({
        entry: { ...parked, status: 'denied' },
      });
      await renderTab();

      expect(await screen.findByText('disconnect_provider')).toBeInTheDocument();
      fireEvent.click(screen.getByRole('button', { name: 'Deny' }));
      expect(adminApi.denyGuardianPendingAction).not.toHaveBeenCalled();

      fireEvent.click(screen.getByRole('button', { name: 'Confirm deny' }));
      await waitFor(() => expect(adminApi.denyGuardianPendingAction).toHaveBeenCalledWith('call-a'));
      expect(await screen.findByText('No calls are waiting.')).toBeInTheDocument();
    });

    it('keep backs out, and a refused denial says why and rereads', async () => {
      vi.mocked(adminApi.getGuardianConfig).mockResolvedValueOnce(sampleResponse());
      vi.mocked(adminApi.listGuardianPendingActions)
        .mockResolvedValueOnce({ entries: [parked], status: 'pending', limit: 50 })
        .mockResolvedValue({ entries: [], status: 'pending', limit: 50 });
      vi.mocked(adminApi.denyGuardianPendingAction).mockRejectedValueOnce({
        response: { status: 404, data: { message: 'That call is no longer waiting' } },
      });
      await renderTab();
      await screen.findByText('disconnect_provider');

      fireEvent.click(screen.getByRole('button', { name: 'Deny' }));
      fireEvent.click(screen.getByRole('button', { name: 'Keep' }));
      expect(screen.getByRole('button', { name: 'Deny' })).toBeInTheDocument();
      expect(adminApi.denyGuardianPendingAction).not.toHaveBeenCalled();

      fireEvent.click(screen.getByRole('button', { name: 'Deny' }));
      fireEvent.click(screen.getByRole('button', { name: 'Confirm deny' }));
      await screen.findByRole('alert');
      await screen.findByText('No calls are waiting.');
    });
  });
});
