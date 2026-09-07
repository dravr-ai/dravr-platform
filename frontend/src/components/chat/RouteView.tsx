// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Draws a hydrated route block as a real MapLibre map over the keyless OpenFreeMap basemap
// ABOUTME: The card under it prints the distance and every climb, so no fact is carried by colour alone

import { useEffect, useMemo, useRef } from 'react';
import type { Map as MapLibreMap } from 'maplibre-gl';
import { useTranslation } from '@pierre/i18n';
import type { RouteClimb, RouteView as RouteViewData } from '@pierre/scene-types';
import { useTheme } from '../../hooks/useTheme';
import {
  addRouteLayers,
  BASEMAP_STYLE,
  climbGeometry,
  routeInk,
  trackGeometry,
  viewportBounds,
} from './routeLayers';

/**
 * A parallel series is index-aligned with the track or it is absent — the
 * photograveur contract is explicit that it is never padded to fit. This
 * re-checks the length anyway, because the series arrives over the wire and a
 * ragged one would print a distance read off the end of the array; a card that
 * drops the kilometre marks is a smaller loss than one that invents them.
 */
function alignedSeries(series: number[] | null, points: number): number[] | null {
  if (series === null || series.length !== points || points === 0) return null;
  return series;
}

/** Metres to the one decimal of a kilometre a route is read in. */
function kilometres(metres: number): string {
  return (metres / 1000).toFixed(1);
}

/** The value at an index the wire supplied, or null when it points nowhere. */
function metresAt(series: number[], index: number): number | null {
  if (!Number.isInteger(index) || index < 0 || index >= series.length) return null;
  return series[index];
}

/** `km 4.0–8.0` for one climb, or null when the track carried no distances. */
function climbRange(distances: number[] | null, climb: RouteClimb): string | null {
  if (distances === null) return null;
  const from = metresAt(distances, climb.start_index);
  const to = metresAt(distances, climb.end_index);
  if (from === null || to === null) return null;
  return `km ${kilometres(from)}–${kilometres(to)}`;
}

/**
 * One recorded track, drawn.
 *
 * The geometry arrives hydrated — the agent emitted an activity reference and
 * the platform read the GPS trace out of the time series — so everything here
 * is presentation: a basemap in the athlete's scheme, the track over it, and
 * the numbers underneath in words a screen reader can read, which the canvas
 * itself can never be.
 */
export default function RouteView({ view }: { view: RouteViewData }) {
  const { t } = useTranslation();
  const { scheme } = useTheme();
  const container = useRef<HTMLDivElement | null>(null);
  const map = useRef<MapLibreMap | null>(null);

  const track = useMemo(() => trackGeometry(view.coordinates), [view.coordinates]);
  const climbs = useMemo(
    () => climbGeometry(view.coordinates, view.climbs),
    [view.coordinates, view.climbs]
  );
  const bounds = useMemo(() => viewportBounds(view.bounds), [view.bounds]);
  const ink = useMemo(() => routeInk(scheme), [scheme]);

  // The style-load handler runs again on every basemap swap and has to paint in
  // the ink the athlete is looking at now, not the ink that was current when it
  // was registered. Declared before the effects that read it so the sync lands
  // first on any render that changes either.
  const paint = useRef({ ink, style: BASEMAP_STYLE[scheme] });
  useEffect(() => {
    paint.current = { ink, style: BASEMAP_STYLE[scheme] };
  }, [ink, scheme]);

  useEffect(() => {
    const node = container.current;
    if (!node) return;

    let live = true;
    let instance: MapLibreMap | null = null;

    void (async () => {
      // Imported here rather than at the top of the module: MapLibre is by some
      // margin the largest dependency the web app has — a megabyte of script
      // and eighty kilobytes of control chrome — and a thread that has never
      // been sent a route must not pay for either. The stylesheet is awaited
      // alongside the code so the zoom stack is never painted unstyled.
      const [{ AttributionControl, Map, NavigationControl }] = await Promise.all([
        import('maplibre-gl'),
        import('maplibre-gl/dist/maplibre-gl.css'),
      ]);
      if (!live) return;

      const created = new Map({
        container: node,
        style: paint.current.style,
        bounds,
        // A track whose extent is a point — an activity that recorded one fix —
        // otherwise fits at the style's maximum zoom, which is a map of one
        // tree. 16 is as close as a route is ever read.
        fitBoundsOptions: { padding: 24, maxZoom: 16 },
        // MapLibre's own attribution lands bottom-right, under the zoom stack;
        // the compact one is docked opposite it below.
        attributionControl: false,
        // The card lives inside a scrolling thread. Cooperative gestures leave
        // a wheel and a one-finger drag to the conversation and ask for
        // ctrl-scroll or two fingers to work the map.
        cooperativeGestures: true,
      });
      map.current = created;
      instance = created;

      created.addControl(new AttributionControl({ compact: true }), 'bottom-left');
      created.addControl(new NavigationControl({ showCompass: false }), 'bottom-right');

      // A style swap discards every source and layer with it, so the track is
      // painted on each style load rather than once after the first.
      created.on('style.load', () => {
        addRouteLayers(created, track, climbs, paint.current.ink);
      });
    })();

    return () => {
      live = false;
      instance?.remove();
      map.current = null;
    };
  }, [bounds, climbs, track]);

  useEffect(() => {
    map.current?.setStyle(BASEMAP_STYLE[scheme]);
  }, [scheme]);

  // A track with no positions is not a map. Both clients say why rather than
  // dropping the block silently: the athlete asked to see a route and is owed
  // the reason there is none.
  if (view.coordinates.length === 0) {
    return <p className="my-4 text-sm text-on-surface-variant">{t('chat.routeNoTrack')}</p>;
  }

  const distances = alignedSeries(view.distances_meters, view.coordinates.length);
  const total = distances === null ? null : metresAt(distances, distances.length - 1);
  const label = view.title
    ? t('chat.routeAltTitled', { title: view.title })
    : t('chat.routeAlt');

  return (
    <figure className="my-4" aria-label={label}>
      {view.title && (
        <figcaption className="mb-2 text-sm font-medium text-on-surface">{view.title}</figcaption>
      )}
      {/* The map's own controls come from MapLibre's stylesheet — third-party
          chrome, deliberately left as it ships, like the provider brand colours
          DESIGN.md §2 exempts. The frame around them is the thread's one card
          recipe: white sheet, hairline, radius 10, no shadow. */}
      <div
        ref={container}
        className="h-64 w-full overflow-hidden rounded-[10px] border ghost-border bg-surface-container-lowest sm:h-80"
      />
      {total !== null && (
        <p className="mt-2 font-mono text-xs text-on-surface">{kilometres(total)} km</p>
      )}
      {view.climbs.length > 0 && (
        <>
          {/* The one line the map draws differently gets named. The track needs
              no legend entry — it is the whole picture — and a legend that
              names both reads as chrome (DESIGN.md §9, Boreal v2.1). */}
          <p className="mt-2 flex items-center gap-1.5 text-xs text-on-surface-variant">
            <span className="w-4 border-t-2 border-dashed border-on-surface" aria-hidden="true" />
            {t('chat.routeClimbs')}
          </p>
          <ul className="mt-1 space-y-0.5 text-xs text-on-surface-variant">
            {view.climbs.map((climb) => {
              const range = climbRange(distances, climb);
              return (
                <li
                  key={`${climb.start_index}-${climb.end_index}`}
                  className="flex flex-wrap items-baseline gap-x-2"
                >
                  <span className="font-medium text-on-surface">
                    {t('chat.routeClimbCategory', { category: climb.category })}
                  </span>
                  <span className="font-mono">{climb.avg_gradient.toFixed(1)}%</span>
                  {range && <span className="font-mono">{range}</span>}
                </li>
              );
            })}
          </ul>
        </>
      )}
      <p className="mt-1 text-xs text-on-surface-variant">source: {view.source_tool}</p>
    </figure>
  );
}
