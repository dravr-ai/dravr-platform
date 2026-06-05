// ABOUTME: Shared Dravr brand mark — the "Stride D" badge as an inline SVG React component.
// ABOUTME: Single source of truth for the in-app logo; replaces the per-screen inline node-graph SVGs.

import { useId } from 'react';

interface DravrLogoProps {
  /** Rendered width/height in px. Defaults to 48 (sidebar size). */
  size?: number;
  className?: string;
}

/**
 * The canonical Dravr mark: a bold D monogram with a forward chevron, on the
 * Boreal forest badge. Self-contained (brings its own background) so it stays
 * legible on light, dark, and green surfaces alike. Gradient ids are namespaced
 * per instance via useId so multiple logos on one page never collide.
 *
 * Rendered as decorative (aria-hidden): every placement sits beside the visible
 * "Dravr" wordmark, so the mark must not be announced a second time.
 */
export function DravrLogo({ size = 48, className }: DravrLogoProps) {
  const uid = useId();
  const bg = `${uid}-bg`;
  const mark = `${uid}-mark`;
  const acc = `${uid}-acc`;
  const clip = `${uid}-clip`;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 1024 1024"
      className={className}
      aria-hidden="true"
      focusable="false"
      xmlns="http://www.w3.org/2000/svg"
    >
      <defs>
        <linearGradient id={bg} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#00261c" />
          <stop offset="1" stopColor="#0e4030" />
        </linearGradient>
        <linearGradient id={mark} x1="0.1" y1="0" x2="0.9" y2="1">
          <stop offset="0" stopColor="#f6f3ea" />
          <stop offset="1" stopColor="#cfe3d6" />
        </linearGradient>
        <linearGradient id={acc} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#bfe6cf" />
          <stop offset="1" stopColor="#79a694" />
        </linearGradient>
        <clipPath id={clip}>
          <rect width="1024" height="1024" rx="230" />
        </clipPath>
      </defs>
      <g clipPath={`url(#${clip})`}>
        <rect width="1024" height="1024" fill={`url(#${bg})`} />
        <circle cx="296" cy="270" r="560" fill="#ffffff" opacity="0.05" />
        <path
          fill={`url(#${mark})`}
          fillRule="evenodd"
          d="M348 288 L520 288 C690 288 800 388 800 512 C800 636 690 736 520 736 L348 736 Z M468 404 L520 404 C610 404 678 448 678 512 C678 576 610 620 520 620 L468 620 Z"
        />
        <path
          fill="none"
          stroke={`url(#${acc})`}
          strokeWidth="46"
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M518 442 L606 512 L518 582"
        />
      </g>
    </svg>
  );
}
