// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Shared Dravr brand mark — the Boreal Ripple mark as a theme-aware image, forest ink on light and mint on dark
// ABOUTME: Single source of truth for the in-app logo; the assets come from frontend/scripts/generate-brand-assets.py

import { clsx } from 'clsx';

interface DravrLogoProps {
  /** Rendered width and height in px. Defaults to 40, the rail size. */
  size?: number;
  className?: string;
}

/** The generated asset edges, smallest first — see frontend/scripts/generate-brand-assets.py. */
const ASSET_EDGES = [96, 192, 512] as const;

/**
 * The smallest generated asset that is at least twice the rendered size, so a
 * 2× display never upsamples; anything past 256px draws from the 512 file.
 */
function markAssetEdge(size: number): (typeof ASSET_EDGES)[number] {
  return ASSET_EDGES.find((edge) => edge >= size * 2) ?? ASSET_EDGES[ASSET_EDGES.length - 1];
}

/**
 * The canonical Dravr mark: a boreal treeline reflected into ripple arcs, the
 * same raster master the phone wears as its app icon. It carries no badge and
 * no gradient — a single ink, forest on the paper surfaces and mint on the dark
 * canvas, swapped by the `.dark` class like every other token.
 *
 * Rendered as decorative (aria-hidden): every placement sits beside the visible
 * DRAVR wordmark or inside chrome that already names the app, so the mark must
 * not be announced a second time.
 */
export function DravrLogo({ size = 40, className }: DravrLogoProps) {
  const edge = markAssetEdge(size);
  const box = { width: size, height: size };
  return (
    <span className={clsx('inline-block shrink-0', className)} style={box} aria-hidden="true">
      <img src={`/brand/mark-ink-${edge}.png`} alt="" width={size} height={size} draggable={false} className="block dark:hidden" />
      <img src={`/brand/mark-mint-${edge}.png`} alt="" width={size} height={size} draggable={false} className="hidden dark:block" />
    </span>
  );
}
