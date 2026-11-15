// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A WebSocket double for the login takeover's component tests, driven as the server would
// ABOUTME: Records what the viewer sent and how it closed; installs the canvas and image decoding jsdom lacks

import { vi } from 'vitest';
import type { SciotteTakeoverInput } from '@pierre/shared-types';

/** One socket the viewer opened, answered by the test as the server. */
export class FakeTakeoverSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  /** Every socket opened since the last {@link installFakeTakeoverSocket}. */
  static instances: FakeTakeoverSocket[] = [];

  readonly url: string;
  readyState = FakeTakeoverSocket.CONNECTING;
  binaryType = 'blob';
  /** The raw text messages the viewer sent. */
  readonly sent: string[] = [];
  /** The codes the viewer closed with, one entry per close. */
  readonly closes: Array<number | undefined> = [];
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;

  constructor(url: string) {
    this.url = url;
    FakeTakeoverSocket.instances.push(this);
  }

  send(data: string): void {
    this.sent.push(data);
  }

  close(code?: number): void {
    this.closes.push(code);
    this.readyState = FakeTakeoverSocket.CLOSED;
  }

  /** What the viewer sent, parsed. */
  inputs(): SciotteTakeoverInput[] {
    return this.sent.map((text) => JSON.parse(text) as SciotteTakeoverInput);
  }

  /** The server accepts the socket. */
  open(): void {
    this.readyState = FakeTakeoverSocket.OPEN;
  }

  /** The server sends a JSON event. */
  event(event: Record<string, unknown>): void {
    this.onmessage?.({ data: JSON.stringify(event) });
  }

  /** The server says the page streams, in a `width` × `height` viewport. */
  ready(width = 400, height = 800): void {
    this.open();
    this.event({ type: 'status', state: 'ready', width, height });
  }

  /** The server sends one frame, `width` × `height` pixels. */
  frame(width = 400, height = 800): void {
    const blob = new Blob([new Uint8Array([0xff, 0xd8, 0xff])], { type: 'image/jpeg' });
    frameSizes.set(blob, { width, height });
    this.onmessage?.({ data: blob });
  }

  /** The server closes the socket. */
  serverClose(): void {
    this.readyState = FakeTakeoverSocket.CLOSED;
    this.onclose?.();
  }
}

/** The size each fake frame decodes to. */
const frameSizes = new WeakMap<Blob, { width: number; height: number }>();

/** Draw calls on any canvas, newest last. */
export const drawImage = vi.fn();

/** The latest socket the viewer opened. */
export function lastSocket(): FakeTakeoverSocket {
  const socket = FakeTakeoverSocket.instances[FakeTakeoverSocket.instances.length - 1];
  if (!socket) throw new Error('no takeover socket was opened');
  return socket;
}

/**
 * jsdom has no PointerEvent, so a fired pointer event would carry no
 * coordinates: a MouseEvent with the pointer's own fields stands in.
 */
class FakePointerEvent extends MouseEvent {
  readonly pointerId: number;
  readonly pointerType: string;
  constructor(type: string, init: PointerEventInit = {}) {
    super(type, init);
    this.pointerId = init.pointerId ?? 1;
    this.pointerType = init.pointerType ?? 'mouse';
  }
}

/** Install the socket, canvas, pointer and image-decoding doubles; call in `beforeEach`. */
export function installFakeTakeoverSocket(): void {
  if (typeof window.PointerEvent === 'undefined') vi.stubGlobal('PointerEvent', FakePointerEvent);
  FakeTakeoverSocket.instances = [];
  drawImage.mockClear();
  vi.stubGlobal('WebSocket', FakeTakeoverSocket);
  vi.stubGlobal(
    'createImageBitmap',
    vi.fn(async (blob: Blob) => ({ ...(frameSizes.get(blob) ?? { width: 1, height: 1 }), close: vi.fn() })),
  );
  HTMLCanvasElement.prototype.getContext = vi.fn(
    () => ({ drawImage }) as unknown as CanvasRenderingContext2D,
  ) as unknown as HTMLCanvasElement['getContext'];
}
