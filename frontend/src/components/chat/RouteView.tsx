// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Draws a hydrated route block as a real MapLibre map over a switchable layer, with a full-screen mode
// ABOUTME: The card under it names every climb in words, so no fact is carried by colour alone

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Maximize2, Minimize2 } from 'lucide-react';
import type { IControl, Map as MapLibreMap, Marker as MapLibreMarker } from 'maplibre-gl';
import { useTranslation, type TFunction } from '@pierre/i18n';
import type { RouteView as RouteViewData } from '@pierre/scene-types';
import {
  alignedSeries,
  climbGeometry,
  climbGradient,
  climbGrade,
  climbRange,
  routeFrame,
  routeMarkers,
  trackGeometry,
  type DistanceUnit,
} from '@pierre/chat-utils';
import {
  DEFAULT_MAP_LAYER,
  MAP_LAYERS,
  ROUTE_INK,
  localizedLabel,
  mapLayerStyle,
  type MapLayer,
} from '@pierre/shared-constants';
import { useTheme } from '../../hooks/useTheme';
import { addRouteLayers } from './routeLayers';
import { addRouteMarkers } from './routeMarkers';

/**
 * MapLibre's own control text, keyed as MapLibre names it, read from the
 * athlete's locale. Only the controls this card docks are listed: the zoom
 * stack, the credit toggle, the canvas label and the cooperative-gesture hint
 * the inline map shows over a bare wheel or one-finger drag. The full-screen
 * toggle is this component's own button, labelled where it is drawn.
 */
function mapLocale(t: TFunction): Record<string, string> {
  return {
    'AttributionControl.ToggleAttribution': t('chat.routeMapCredits'),
    'Map.Title': t('chat.routeAlt'),
    'NavigationControl.ZoomIn': t('chat.routeMapZoomIn'),
    'NavigationControl.ZoomOut': t('chat.routeMapZoomOut'),
    'CooperativeGesturesHandler.MacHelpText': t('chat.routeMapScrollMac'),
    'CooperativeGesturesHandler.WindowsHelpText': t('chat.routeMapScrollWindows'),
    'CooperativeGesturesHandler.MobileHelpText': t('chat.routeMapTwoFingers'),
  };
}

/**
 * How the map is being shown. `native` is the browser's Fullscreen API;
 * `overlay` is the stand-in where the API is missing — an iPhone's Safari
 * exposes no element fullscreen — a fixed sheet over the page that Esc closes.
 */
type Screen = 'inline' | 'native' | 'overlay';

function layerById(id: string): MapLayer {
  return MAP_LAYERS.find((layer) => layer.id === id) ?? MAP_LAYERS[0];
}

/**
 * Relabels a loaded basemap's places in the athlete's language — OpenFreeMap's
 * styles label them in English first ({@link localizedLabel}). A raster
 * layer's style has no symbol layers, so it is left as it is.
 */
function labelInLanguage(map: MapLibreMap, language: string): void {
  for (const layer of map.getStyle().layers) {
    if (layer.type !== 'symbol') continue;
    const field = layer.layout?.['text-field'];
    if (field === undefined) continue;
    const localized = localizedLabel(field, language);
    if (localized !== field) map.setLayoutProperty(layer.id, 'text-field', localized);
  }
}

/** Builds the attribution control, once MapLibre's module has been loaded. */
type CreditFactory = (options: { compact: boolean }) => IControl;

/**
 * Dock the credit for `layer` bottom-left, replacing the one there.
 *
 * A basemap style's credit is compact and folded: on a phone-width card the
 * open pill covered the lower third of the route, and it stays one tap away
 * behind its button. An Esri layer's credit is never folded — the imagery's
 * sources stay on the map whenever the imagery is — so a raster layer gets the
 * plain control, which MapLibre keeps open at every width and never collapses
 * on a drag.
 */
function dockCredit(
  instance: MapLibreMap,
  create: CreditFactory,
  current: IControl | null,
  layer: MapLayer
): IControl {
  if (current) instance.removeControl(current);
  const control = create({ compact: layer.kind === 'style' });
  instance.addControl(control, 'bottom-left');
  return control;
}

/** Fold a compact credit that MapLibre opened expanded. */
function foldCredit(node: HTMLElement): void {
  node.querySelector('.maplibregl-ctrl-attrib')?.classList.remove('maplibregl-compact-show');
}

/**
 * The legend's climb swatch, drawn in the map's own three layers — casing,
 * orange track, near-black dash — so the legend and the overlay cannot say
 * different things about which line is which, on either card surface.
 */
function ClimbSwatch() {
  return (
    <svg width="18" height="8" viewBox="0 0 18 8" aria-hidden="true">
      <line x1="1" y1="4" x2="17" y2="4" stroke={ROUTE_INK.casing} strokeWidth="6" strokeLinecap="round" />
      <line x1="1" y1="4" x2="17" y2="4" stroke={ROUTE_INK.track} strokeWidth="3" strokeLinecap="round" />
      <line x1="1" y1="4" x2="17" y2="4" stroke={ROUTE_INK.climb} strokeWidth="3.4" strokeDasharray="4 3" />
    </svg>
  );
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
export default function RouteView({
  view,
  compact = false,
  markerUnit = null,
}: {
  view: RouteViewData;
  /** A shorter inline frame, for a map in a side panel rather than a reading column. */
  compact?: boolean;
  /**
   * Draw the start, the finish and a distance mark counted in this unit — a
   * recorded on-foot activity's map. Null, the default, draws the track alone,
   * as a suggested route in the chat is drawn.
   */
  markerUnit?: DistanceUnit | null;
}) {
  const { t, language } = useTranslation();
  const { scheme } = useTheme();
  const container = useRef<HTMLDivElement | null>(null);
  const figure = useRef<HTMLElement | null>(null);
  const map = useRef<MapLibreMap | null>(null);

  const track = useMemo(() => trackGeometry(view.coordinates), [view.coordinates]);
  const climbs = useMemo(
    () => climbGeometry(view.coordinates, view.climbs),
    [view.coordinates, view.climbs]
  );
  const bounds = useMemo(() => routeFrame(view.bounds), [view.bounds]);
  const markers = useMemo(
    () => (markerUnit === null ? [] : routeMarkers(view.coordinates, view.distances_meters, markerUnit)),
    [markerUnit, view.coordinates, view.distances_meters]
  );
  const stage = useRef<HTMLDivElement | null>(null);
  // Every map opens on the default layer; a pick holds for this map only (carnet#699).
  const [layerId, pickLayer] = useState(DEFAULT_MAP_LAYER);
  const [screen, setScreen] = useState<Screen>('inline');
  const layer = layerById(layerId);
  const style = useMemo(() => mapLayerStyle(layer, scheme), [layer, scheme]);
  // The layer drawn now, for handlers registered once per map; and the credit
  // control docked for it, swapped when the layer's kind changes.
  const currentLayer = useRef(layer);
  const credit = useRef<{ control: IControl; kind: MapLayer['kind']; create: CreditFactory } | null>(
    null
  );

  // The map is built once per track, but must open on the style the athlete
  // is looking at now, not the one current when the effect was registered.
  // Declared before the effects that read it so the sync lands first.
  const initialStyle = useRef(style);
  // MapLibre reads its control text once, at construction; the map is built
  // once per track, so it speaks the athlete's language as of that build.
  const locale = useRef(mapLocale(t));
  // The language each style load labels the basemap in, read when it loads.
  const labelLanguage = useRef(language);
  useEffect(() => {
    initialStyle.current = style;
    currentLayer.current = layer;
    locale.current = mapLocale(t);
    labelLanguage.current = language;
  }, [style, layer, t, language]);

  useEffect(() => {
    const node = container.current;
    if (!node) return;
    // The figure the map sits in, rendered with it.
    const frame = figure.current;

    let live = true;
    let instance: MapLibreMap | null = null;
    let pinned: MapLibreMarker[] = [];

    void (async () => {
      // Imported here rather than at the top of the module: MapLibre is by some
      // margin the largest dependency the web app has — a megabyte of script
      // and eighty kilobytes of control chrome — and a thread that has never
      // been sent a route must not pay for either. The stylesheet is awaited
      // alongside the code so the zoom stack is never painted unstyled.
      const [{ AttributionControl, Map, Marker, NavigationControl, setWorkerUrl }, { default: workerUrl }] =
        await Promise.all([
          import('maplibre-gl'),
          // MapLibre 6 looks for its tile worker beside its own module, and a
          // bundle has no such sibling: dev pre-bundles MapLibre into
          // `.vite/deps` and the build renames it into `assets/`, so the
          // request falls through to index.html, the worker dies on its first
          // `<`, and no tile is ever drawn. Vite bundles the worker with its
          // shared chunk and hands back its URL.
          import('maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url'),
          import('maplibre-gl/dist/maplibre-gl.css'),
        ]);
      if (!live) return;
      setWorkerUrl(workerUrl);

      const created = new Map({
        container: node,
        style: initialStyle.current,
        // Handed over at construction rather than fitted afterwards, so the
        // first frame is already over the route instead of panning to it once
        // tiles arrive. A one-fix track arrives widened to the shared minimum
        // span, so it opens at the distance the phone frames it from.
        bounds,
        fitBoundsOptions: { padding: 24 },
        // MapLibre's own attribution lands bottom-right, under the zoom stack;
        // the compact one is docked opposite it below.
        attributionControl: false,
        // The card lives inside a scrolling thread. Cooperative gestures leave
        // a wheel and a one-finger drag to the conversation and ask for
        // ctrl-scroll or two fingers to work the map.
        cooperativeGestures: true,
        locale: locale.current,
      });
      map.current = created;
      instance = created;

      const create: CreditFactory = (options) => new AttributionControl(options);
      const opening = currentLayer.current;
      credit.current = {
        control: dockCredit(created, create, null, opening),
        kind: opening.kind,
        create,
      };
      created.addControl(new NavigationControl({ showCompass: false }), 'bottom-right');
      // DOM markers sit above the canvas, outside the style, so a layer swap
      // keeps them and the keyless imagery — which has no glyphs to set a
      // symbol layer's numbers in — carries them as the basemap does.
      pinned = addRouteMarkers(created, Marker, markers);
      // The compact credit opens itself expanded on load; a basemap's is
      // folded on the map's one load. An imagery credit stays open.
      created.on('load', () => {
        if (currentLayer.current.kind === 'style') foldCredit(node);
      });

      // A style swap discards every source and layer with it, so the track is
      // painted on each style load rather than once after the first. The
      // figure says once it is — `data-route-drawn` — since the canvas alone
      // cannot tell a drawn route from an empty map, which is what the
      // athlete got when the tile worker could not load.
      created.on('style.load', () => {
        labelInLanguage(created, labelLanguage.current);
        addRouteLayers(created, track, climbs);
        frame?.setAttribute('data-route-drawn', 'true');
      });
    })();

    return () => {
      live = false;
      for (const marker of pinned) marker.remove();
      instance?.remove();
      map.current = null;
      credit.current = null;
      frame?.removeAttribute('data-route-drawn');
    };
  }, [bounds, climbs, markers, track]);

  // A scheme flip or a layer pick swaps the style; the route repaints on the
  // style load that follows. The first run is the style the map was built on.
  const shown = useRef(style);
  useEffect(() => {
    if (shown.current === style) return;
    shown.current = style;
    map.current?.setStyle(style);
  }, [style]);

  // Moving between a basemap and imagery re-docks the credit in the form the
  // new layer requires: folded for a basemap, open for imagery.
  useEffect(() => {
    const instance = map.current;
    const docked = credit.current;
    const node = container.current;
    if (!instance || !docked || !node || docked.kind === layer.kind) return;
    credit.current = {
      ...docked,
      control: dockCredit(instance, docked.create, docked.control, layer),
      kind: layer.kind,
    };
    if (layer.kind === 'style') foldCredit(node);
  }, [layer]);

  // The browser owns native fullscreen, and Esc leaves it without asking the
  // page, so the mode is read back from the document rather than assumed.
  useEffect(() => {
    const sync = () => {
      setScreen((current) =>
        document.fullscreenElement === stage.current
          ? 'native'
          : current === 'native'
            ? 'inline'
            : current
      );
    };
    document.addEventListener('fullscreenchange', sync);
    return () => document.removeEventListener('fullscreenchange', sync);
  }, []);

  // The overlay has no browser behind it, so Esc is the page's to honour.
  useEffect(() => {
    if (screen !== 'overlay') return;
    const close = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setScreen('inline');
    };
    document.addEventListener('keydown', close);
    return () => document.removeEventListener('keydown', close);
  }, [screen]);

  // Full screen, the map is the page: a wheel and a one-finger drag work it
  // directly. Inline, they belong to the thread the card scrolls in. Either
  // way the route is framed again for the new size — kept at the card's zoom,
  // a full-screen run is a small ring in the middle of a city.
  useEffect(() => {
    const instance = map.current;
    if (!instance) return;
    if (screen === 'inline') instance.cooperativeGestures.enable();
    else instance.cooperativeGestures.disable();
    instance.resize();
    instance.fitBounds(bounds, { padding: screen === 'inline' ? 24 : 64, animate: false });
  }, [screen, bounds]);

  const toggleScreen = useCallback(() => {
    if (screen === 'native') {
      void document.exitFullscreen();
      return;
    }
    if (screen === 'overlay') {
      setScreen('inline');
      return;
    }
    const node = stage.current;
    if (node && document.fullscreenEnabled && typeof node.requestFullscreen === 'function') {
      // A refused request (a sandboxed frame, a denied permission) still owes
      // the athlete a bigger map, so it lands on the overlay instead.
      node.requestFullscreen().catch(() => setScreen('overlay'));
    } else {
      setScreen('overlay');
    }
  }, [screen]);

  // A track with no positions is not a map. Both clients say why rather than
  // dropping the block silently: the athlete asked to see a route and is owed
  // the reason there is none.
  if (view.coordinates.length === 0) {
    return <p className="my-4 text-sm text-on-surface-variant">{t('chat.routeNoTrack')}</p>;
  }

  // Read for the climbs' ranges only. The series is measured along the drawn
  // GPS line and ends where the privacy trim cuts it, so its last value is not
  // the activity's distance and is never printed as one.
  const distances = alignedSeries(view.distances_meters, view.coordinates.length);
  const label = view.title
    ? t('chat.routeAltTitled', { title: view.title })
    : t('chat.routeAlt');

  return (
    <figure ref={figure} className="my-4" aria-label={label}>
      {view.title && (
        <figcaption className="mb-2 text-sm font-medium text-on-surface">{view.title}</figcaption>
      )}
      {/* The map's own controls come from MapLibre's stylesheet — third-party
          chrome, deliberately left as it ships, like the provider brand colours
          DESIGN.md §2 exempts. The frame around them is the thread's one card
          recipe: white sheet, hairline, radius 10, no shadow. */}
      <div
        ref={stage}
        data-map-screen={screen}
        className={
          screen === 'overlay'
            ? 'fixed inset-0 z-50 bg-surface-container-lowest'
            : screen === 'native'
              ? 'relative h-full w-full bg-surface-container-lowest'
              : 'relative'
        }
      >
        {/* The credit's corner stops short of the zoom stack docked bottom-right
            (29px wide, 10px in from the edge), so an open credit wraps beside
            the buttons instead of running under them: at phone width the Esri
            line, which must stay readable, otherwise lost its last words. The
            credit sits on MapLibre's translucent white pill in both schemes, so
            its ink is MapLibre's own dark one, not the page's on-surface ink,
            which is near-white in the dark theme. */}
        <div
          ref={container}
          className={`[&_.maplibregl-ctrl-bottom-left]:right-12 [&_.maplibregl-ctrl-attrib]:text-black/75 ${
            screen === 'inline'
              ? `${compact ? 'h-48' : 'h-64 sm:h-80'} w-full overflow-hidden rounded-[10px] border ghost-border bg-surface-container-lowest`
              : 'h-full w-full'
          }`}
        />
        {/* One bar carries both controls, so the switcher reserves the
            full-screen button's width instead of sliding under it: on a card
            too narrow for the two side by side (a chat card can be under 200px)
            the button wraps to its own row, still in the corner, and the
            switcher wraps its own options rather than clip a label. The bar
            lets a drag through to the map wherever it holds no control. */}
        <div
          data-map-controls
          className="pointer-events-none absolute inset-x-2 top-2 flex flex-wrap items-start gap-2"
        >
          <div
            role="group"
            data-map-control
            aria-label={t('chat.routeLayers')}
            className="pointer-events-auto flex max-w-full flex-wrap overflow-hidden rounded-[10px] border ghost-border bg-surface-container-lowest text-xs"
          >
            {MAP_LAYERS.map((option) => {
              const active = option.id === layerId;
              return (
                <button
                  key={option.id}
                  type="button"
                  aria-pressed={active}
                  onClick={() => pickLayer(option.id)}
                  className={`whitespace-nowrap px-2 py-1.5 font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-primary ${
                    active ? 'bg-primary text-on-primary' : 'text-on-surface hover:bg-surface-container'
                  }`}
                >
                  {t(option.labelKey)}
                </button>
              );
            })}
          </div>
          <button
            type="button"
            onClick={toggleScreen}
            data-map-control
            aria-label={screen === 'inline' ? t('chat.routeFullScreen') : t('chat.routeExitFullScreen')}
            title={screen === 'inline' ? t('chat.routeFullScreen') : t('chat.routeExitFullScreen')}
            className="pointer-events-auto ml-auto flex h-8 w-8 shrink-0 items-center justify-center rounded-[10px] border ghost-border bg-surface-container-lowest text-on-surface hover:bg-surface-container focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          >
            {screen === 'inline' ? (
              <Maximize2 className="h-4 w-4" aria-hidden="true" />
            ) : (
              <Minimize2 className="h-4 w-4" aria-hidden="true" />
            )}
          </button>
        </div>
      </div>
      {view.climbs.length > 0 && (
        <>
          {/* The one line the map draws differently gets named. The track needs
              no legend entry — it is the whole picture — and a legend that
              names both reads as chrome (DESIGN.md §9, Boreal v2.1). */}
          <p className="mt-2 flex items-center gap-1.5 text-xs text-on-surface-variant">
            <ClimbSwatch />
            {t('chat.routeClimbs')}
          </p>
          <ul className="mt-1 space-y-0.5 text-xs text-on-surface-variant">
            {view.climbs.map((climb) => {
              const range = climbRange(distances, climb, language);
              const grade = climbGrade(climb, t);
              return (
                <li
                  key={`${climb.start_index}-${climb.end_index}`}
                  className="flex flex-wrap items-baseline gap-x-2"
                >
                  {grade !== null && <span className="font-medium text-on-surface">{grade}</span>}
                  <span className="font-mono">{climbGradient(climb, language)}</span>
                  {range && <span className="font-mono">{range}</span>}
                </li>
              );
            })}
          </ul>
        </>
      )}
    </figure>
  );
}
