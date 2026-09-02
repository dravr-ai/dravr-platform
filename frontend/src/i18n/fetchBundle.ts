// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The web app's reader of the live string catalogue, handed to initI18n
// ABOUTME: Next to the locale persister, so both directions of the locale contract live in one place

import type { BundleFetcher } from '@pierre/i18n';
import { i18nApi } from '../services/api';

/** Read one locale's live catalogue through `GET /api/i18n/{locale}`. */
export const fetchBundle: BundleFetcher = (locale, etag) => i18nApi.bundle(locale, etag);
