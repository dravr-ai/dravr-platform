// ABOUTME: Shared Dravr brand mark — the "Momentum" badge as an inline SVG React component.
// ABOUTME: Single source of truth for the in-app logo; replaces the per-screen inline node-graph SVGs.

import { useId } from 'react';

interface DravrLogoProps {
  /** Rendered width/height in px. Defaults to 48 (sidebar size). */
  size?: number;
  className?: string;
}

/**
 * The canonical Dravr mark: three upward "momentum" ribbons rising to a node, on
 * the Boreal forest badge. Self-contained (brings its own background) so it stays
 * legible on light, dark, and green surfaces alike. Gradient ids are namespaced
 * per instance via useId so multiple logos on one page never collide.
 *
 * Rendered as decorative (aria-hidden): every placement sits beside the visible
 * "Dravr" wordmark, so the mark must not be announced a second time.
 */
export function DravrLogo({ size = 48, className }: DravrLogoProps) {
  const uid = useId();
  const bg = `${uid}-bg`;
  const r1 = `${uid}-r1`;
  const r2 = `${uid}-r2`;
  const r3 = `${uid}-r3`;
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
        <linearGradient id={r1} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#5e7a82" stopOpacity="0.25" />
          <stop offset="1" stopColor="#9bb3bb" />
        </linearGradient>
        <linearGradient id={r2} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#79a694" stopOpacity="0.3" />
          <stop offset="1" stopColor="#b6e0c9" />
        </linearGradient>
        <linearGradient id={r3} x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#8f6a2e" stopOpacity="0.35" />
          <stop offset="1" stopColor="#e9d2a3" />
        </linearGradient>
        <clipPath id={clip}>
          <rect width="1024" height="1024" rx="230" />
        </clipPath>
      </defs>
      <g clipPath={`url(#${clip})`}>
        <rect width="1024" height="1024" fill={`url(#${bg})`} />
        <circle cx="720" cy="300" r="520" fill="#ffffff" opacity="0.05" />
        {size <= 48 ? (
          // Compact: two bold ribbons + a larger node — stays crisp at favicon size.
          <>
            <g fill="none" strokeLinecap="round">
              <path stroke="#9fd4ba" strokeWidth="122" d="M250 650 C470 630 580 540 770 330" />
              <path stroke="#e3c489" strokeWidth="96" d="M300 800 C500 788 585 705 715 540" />
            </g>
            <circle cx="792" cy="300" r="86" fill="#f4f1e8" />
          </>
        ) : (
          <>
            <g fill="none" strokeLinecap="round">
              <path stroke={`url(#${r1})`} strokeWidth="70" d="M230 660 C420 660 520 600 720 380" />
              <path stroke={`url(#${r2})`} strokeWidth="84" d="M250 540 C460 540 600 470 800 250" />
              <path stroke={`url(#${r3})`} strokeWidth="58" d="M300 780 C460 780 540 730 700 560" />
            </g>
            <circle cx="800" cy="250" r="46" fill="#f4f1e8" />
          </>
        )}
      </g>
    </svg>
  );
}
