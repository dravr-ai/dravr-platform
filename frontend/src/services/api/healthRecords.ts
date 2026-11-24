// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web's health records API — served records by kind, the lab review queue, uploaded lab reports
// ABOUTME: Built here, not in createPierreApi or the barrel: only the lazy settings chunk reaches it, so it stays out of the entry

import { createHealthRecordsApi } from '@pierre/api-client';
import { axios } from './client';

// The domain and its response parsers are only reachable through this module,
// which only the lazily loaded settings screen imports. Exporting it from
// `./index` (which the entry loads) would put them back before first paint.
export const healthRecordsApi = createHealthRecordsApi(axios);
