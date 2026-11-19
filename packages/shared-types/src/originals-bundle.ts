// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The originals bundle wire shapes — a bundle's state as the export and its routes name it, and a signed download
// ABOUTME: With their parsers: a body of any other shape reads as null, never as a half-filled bundle

/** Where a bundle's build stands. */
export type OriginalsBundleStatus = 'building' | 'ready' | 'failed';

/** The athlete's originals bundle (`/api/user/export/originals`, and the export's `originals`). */
export interface OriginalsBundle {
  bundleId: string;
  status: OriginalsBundleStatus;
  /** Sealed with a passphrase the server never kept. */
  sealed: boolean;
  createdAt: string;
  /** When it stops being downloadable. */
  expiresAt: string;
  /** Files it holds, once ready. */
  files: number | null;
  /** Files the store no longer held, named in its manifest, once ready. */
  missing: number | null;
  /** Files whose bytes did not match their digest, once ready. */
  damaged: number | null;
  /** The download's size in bytes, once ready. */
  byteSize: number | null;
  /** The download's SHA-256, once ready. */
  sha256: string | null;
  /** The route the athlete downloads it from, once ready. */
  download: string | null;
}

/** A short signed URL the bundle downloads from, straight from storage. */
export interface OriginalsSignedDownload {
  url: string;
  expiresAt: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isNullableCount(value: unknown): value is number | null {
  return value === null || value === undefined || (typeof value === 'number' && Number.isInteger(value) && value >= 0);
}

function isNullableString(value: unknown): value is string | null {
  return value === null || value === undefined || typeof value === 'string';
}

const STATUSES: readonly OriginalsBundleStatus[] = ['building', 'ready', 'failed'];

/** Read a bundle, or `null` for any other shape. */
export function parseOriginalsBundle(body: unknown): OriginalsBundle | null {
  if (!isRecord(body)) return null;
  const status = body.status;
  if (
    typeof body.bundle_id !== 'string' ||
    typeof status !== 'string' ||
    !STATUSES.includes(status as OriginalsBundleStatus) ||
    typeof body.sealed !== 'boolean' ||
    typeof body.created_at !== 'string' ||
    typeof body.expires_at !== 'string' ||
    !isNullableCount(body.files) ||
    !isNullableCount(body.missing) ||
    !isNullableCount(body.damaged) ||
    !isNullableCount(body.byte_size) ||
    !isNullableString(body.sha256) ||
    !isNullableString(body.download)
  ) {
    return null;
  }
  return {
    bundleId: body.bundle_id,
    status: status as OriginalsBundleStatus,
    sealed: body.sealed,
    createdAt: body.created_at,
    expiresAt: body.expires_at,
    files: body.files ?? null,
    missing: body.missing ?? null,
    damaged: body.damaged ?? null,
    byteSize: body.byte_size ?? null,
    sha256: body.sha256 ?? null,
    download: body.download ?? null,
  };
}

/** Read a signed download, or `null` for any other shape. */
export function parseOriginalsSignedDownload(body: unknown): OriginalsSignedDownload | null {
  if (!isRecord(body) || typeof body.url !== 'string' || body.url === '') return null;
  if (typeof body.expires_at !== 'string') return null;
  return { url: body.url, expiresAt: body.expires_at };
}
