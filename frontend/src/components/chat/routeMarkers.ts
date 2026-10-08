// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The start, finish and distance markers a recorded on-foot route is drawn with, as MapLibre DOM markers
// ABOUTME: DOM rather than a symbol layer, so they survive a layer swap and need no glyphs on the keyless imagery

import type { Map as MapLibreMap, Marker as MapLibreMarker, MarkerOptions } from 'maplibre-gl';
import type { RouteMarker } from '@pierre/chat-utils';
import { ROUTE_INK } from '@pierre/shared-constants';

/** MapLibre's `Marker` constructor, handed in once its module has been loaded. */
export type MarkerFactory = new (options: MarkerOptions) => MapLibreMarker;

/**
 * The element one marker is drawn as.
 *
 * The start is a round dot and the finish a square — the shape, not only the
 * fill, tells them apart — both ringed in the casing white that edges the
 * track. A distance mark is the unit count on a white disc ringed in the
 * track's orange. Every marker is hidden from assistive technology: the map
 * is one labelled image, and its markers are part of that picture, like the
 * line they sit on.
 */
export function markerElement(marker: RouteMarker): HTMLElement {
  const element = document.createElement('div');
  element.setAttribute('aria-hidden', 'true');
  element.dataset.routeMarker = marker.kind;
  element.style.boxSizing = 'border-box';
  element.style.pointerEvents = 'none';
  if (marker.kind === 'distance') {
    element.textContent = String(marker.units);
    element.style.minWidth = '20px';
    element.style.height = '20px';
    element.style.padding = '0 3px';
    element.style.borderRadius = '10px';
    element.style.border = `2px solid ${ROUTE_INK.track}`;
    element.style.background = ROUTE_INK.casing;
    element.style.color = ROUTE_INK.markText;
    // The interface's smallest step (DESIGN.md: nothing under text-xs).
    element.style.font = '700 12px/16px system-ui, sans-serif';
    element.style.textAlign = 'center';
    return element;
  }
  element.style.width = '14px';
  element.style.height = '14px';
  element.style.border = `2px solid ${ROUTE_INK.casing}`;
  element.style.background = marker.kind === 'start' ? ROUTE_INK.start : ROUTE_INK.finish;
  element.style.borderRadius = marker.kind === 'start' ? '50%' : '2px';
  return element;
}

/**
 * Pin every marker onto the map and hand back what was added, for removal.
 *
 * The distance marks go on first, then the finish, then the start, so on a
 * loop whose finish lands on its start the start is the one on top.
 */
export function addRouteMarkers(
  map: MapLibreMap,
  Marker: MarkerFactory,
  markers: RouteMarker[]
): MapLibreMarker[] {
  const order = { distance: 0, finish: 1, start: 2 } as const;
  return [...markers]
    .sort((a, b) => order[a.kind] - order[b.kind])
    .map((marker) => {
      const [latitude, longitude] = marker.position;
      return new Marker({ element: markerElement(marker), anchor: 'center' })
        .setLngLat([longitude, latitude])
        .addTo(map);
    });
}
