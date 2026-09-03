// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Personas domain API — the « Style de coaching » cards, rendered from the live contract registry
// ABOUTME: One method; the server decides every word on the card, including which rules the contract sets

import type { AxiosInstance } from 'axios';
import type { PersonaCard, PersonaRule, PersonasResponse } from '@pierre/shared-types';
import { ENDPOINTS } from '../core/endpoints';

export type { PersonaCard, PersonaRule, PersonasResponse };

/**
 * Creates the personas API bound to an axios instance.
 */
export function createPersonasApi(axios: AxiosInstance) {
  return {
    /**
     * The persona cards, in enum order.
     *
     * `locale` overrides the account's stored language — pass the one the app
     * is currently rendering in, so the cards follow a language switch without
     * waiting for the account to be saved. The server falls back to the stored
     * locale, then to English.
     */
    async list(locale?: string): Promise<PersonasResponse> {
      const response = await axios.get<PersonasResponse>(ENDPOINTS.PERSONAS.LIST, {
        params: locale ? { locale } : undefined,
      });
      return response.data;
    },
  };
}

export type PersonasApi = ReturnType<typeof createPersonasApi>;
