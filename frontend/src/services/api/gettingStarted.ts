// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home's Getting started read — connect your data, then the four first-run flows (carnet#1099)
// ABOUTME: Web-only until native Home gets the section (registre#1102); then it moves to @pierre/api-client

import { parseGettingStartedResponse, type GettingStartedResponse } from '@pierre/shared-types';
import { axios } from './client';

const GETTING_STARTED = '/api/me/getting-started';

export const gettingStartedApi = {
  /**
   * The rows of Getting started, each done, current or waiting on a
   * prerequisite, with the postback an open row sends into the Home thread.
   */
  async get(): Promise<GettingStartedResponse> {
    const response = await axios.get<unknown>(GETTING_STARTED);
    const parsed = parseGettingStartedResponse(response.data);
    if (parsed === null) {
      throw new Error(`Unexpected response shape from ${GETTING_STARTED}`);
    }
    return parsed;
  },
};
