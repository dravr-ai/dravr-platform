// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the data access requests domain — the routes and bodies each step sends, and the shape check on every answer (carnet#1646)
// ABOUTME: Runs the real domain over a stub axios, so web and mobile share what is pinned here

import { describe, expect, it, vi } from 'vitest';
import type { AxiosInstance } from 'axios';
import { createDataAccessRequestsApi } from '../src/domains/dataAccessRequests';

const REQUEST = {
  id: 'r 1',
  reference: 'DRV-2026-AB3XZ7',
  controller: 'Acme Fitness',
  controller_contact: 'privacy@acme.example',
  athlete_identifier: 'ana@example.com',
  law: 'quebec_p39_1',
  scope: 'access',
  letter_locale: 'fr',
  status: 'sent',
  derived_state: null,
  sent_on: '2026-11-08',
  due_on: '2026-12-08',
  extended_until: null,
  answered_on: null,
  created_at: '2026-11-08T15:00:00Z',
  updated_at: '2026-11-08T15:00:00Z',
};

function stubAxios(data: unknown = REQUEST) {
  const get = vi.fn().mockResolvedValue({ data });
  const post = vi.fn().mockResolvedValue({ data });
  const patch = vi.fn().mockResolvedValue({ data });
  return { get, post, patch, api: createDataAccessRequestsApi({ get, post, patch } as unknown as AxiosInstance) };
}

describe('dataAccessRequests domain', () => {
  it('lists the requests out of their envelope', async () => {
    const { get, api } = stubAxios({ requests: [REQUEST] });
    await expect(api.list()).resolves.toHaveLength(1);
    expect(get).toHaveBeenCalledWith('/api/user/data-access-requests');
  });

  it('creates a draft with the body it is given', async () => {
    const { post, api } = stubAxios();
    const body = {
      controller: 'Acme Fitness',
      controller_contact: 'privacy@acme.example',
      athlete_identifier: 'ana@example.com',
      law: 'gdpr' as const,
      scope: 'access' as const,
      letter_locale: 'en' as const,
    };
    await api.create(body);
    expect(post).toHaveBeenCalledWith('/api/user/data-access-requests', body);
  });

  it('edits a draft with PATCH on its encoded id', async () => {
    const { patch, api } = stubAxios();
    await api.edit('r 1', { controller: 'Acme' });
    expect(patch).toHaveBeenCalledWith('/api/user/data-access-requests/r%201', { controller: 'Acme' });
  });

  it('posts each step with its own body', async () => {
    const { post, api } = stubAxios();
    await api.markSent('r1', '2026-11-08');
    expect(post).toHaveBeenLastCalledWith('/api/user/data-access-requests/r1/sent', { sent_on: '2026-11-08' });
    await api.recordExtension('r1', '2027-01-20');
    expect(post).toHaveBeenLastCalledWith('/api/user/data-access-requests/r1/extension', {
      extended_until: '2027-01-20',
    });
    await api.markAnswered('r1', '2026-11-20');
    expect(post).toHaveBeenLastCalledWith('/api/user/data-access-requests/r1/answered', { answered_on: '2026-11-20' });
    await api.close('r1');
    expect(post).toHaveBeenLastCalledWith('/api/user/data-access-requests/r1/close', {});
  });

  it('asks for the letter in a language only when one is named', async () => {
    const letter = { locale: 'en', subject: 's', body: 'b', text: 's\n\nb\n', filename: 'DRV.txt', mailto: null };
    const { get, api } = stubAxios(letter);
    await api.letter('r1');
    expect(get).toHaveBeenLastCalledWith('/api/user/data-access-requests/r1/letter', undefined);
    await api.letter('r1', 'de');
    expect(get).toHaveBeenLastCalledWith('/api/user/data-access-requests/r1/letter', { params: { locale: 'de' } });
  });

  it('fails an answer of another shape, naming the endpoint', async () => {
    const { api } = stubAxios({ id: 'r1' });
    await expect(api.get('r1')).rejects.toThrow('/api/user/data-access-requests/r1');
  });
});
