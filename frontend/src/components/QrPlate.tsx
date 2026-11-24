// ABOUTME: A server-rendered QR code on a white plate, so a phone camera can scan it in either theme
// ABOUTME: Shared by every chat-app link the web offers: a person's own channel link and a group's chat connection

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/**
 * Draw a QR the server rendered as SVG markup. The SVG travels as a data URI
 * inside an `<img>`, never as inline markup, so nothing in it can run.
 */
export default function QrPlate({ svg, alt, ...rest }: { svg: string; alt: string; 'data-testid'?: string }) {
  return (
    // White plate so the black-on-white QR stays scannable in any theme.
    <div className="rounded-xl bg-white p-3" data-testid={rest['data-testid']}>
      <img src={`data:image/svg+xml;utf8,${encodeURIComponent(svg)}`} alt={alt} className="h-44 w-44" />
    </div>
  );
}
