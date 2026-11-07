// ABOUTME: Tests for CreateA2AClient — a client registers with no capability selected
// ABOUTME: Pins that capabilities are optional labels, as the server treats them, and that the labels chosen are sent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import CreateA2AClient from '../CreateA2AClient';

const registerA2AClient = vi.fn();

vi.mock('../../services/api', () => ({
  a2aApi: {
    registerA2AClient: (...args: unknown[]) => registerA2AClient(...args),
  },
}));

function renderForm() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const { container } = render(
    <QueryClientProvider client={queryClient}>
      <CreateA2AClient />
    </QueryClientProvider>,
  );
  return container;
}

function fillRequiredFields(container: HTMLElement) {
  const fields = container.querySelectorAll<HTMLInputElement | HTMLTextAreaElement>('[required]');
  fields.forEach((field) => {
    const value = field.getAttribute('type') === 'email' ? 'dev@example.com' : 'Training Planner';
    fireEvent.change(field, { target: { value } });
  });
}

function submitButton(container: HTMLElement): HTMLButtonElement {
  const button = container.querySelector<HTMLButtonElement>('button[type="submit"]');
  if (!button) throw new Error('the form has a submit button');
  return button;
}

describe('CreateA2AClient', () => {
  beforeEach(() => {
    registerA2AClient.mockReset();
    registerA2AClient.mockResolvedValue({
      client_id: 'a2a_client_1',
      client_secret: 'a-secret',
    });
  });

  it('registers a client with no capability selected', async () => {
    const container = renderForm();
    fillRequiredFields(container);

    const submit = submitButton(container);
    expect(submit.disabled).toBe(false);
    fireEvent.submit(submit.closest('form') as HTMLFormElement);

    await waitFor(() => expect(registerA2AClient).toHaveBeenCalledTimes(1));
    expect(registerA2AClient.mock.calls[0][0].capabilities).toEqual([]);
  });

  it('sends the capabilities chosen', async () => {
    const container = renderForm();
    fillRequiredFields(container);
    const name = i18n.t('a2a.capProviderName');
    fireEvent.click(screen.getByRole('checkbox', { name }));
    fireEvent.submit(submitButton(container).closest('form') as HTMLFormElement);

    await waitFor(() => expect(registerA2AClient).toHaveBeenCalledTimes(1));
    expect(registerA2AClient.mock.calls[0][0].capabilities).toEqual(['provider-integration']);
  });
});
