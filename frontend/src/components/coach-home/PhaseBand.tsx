// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: One phase band whose name shows only when the whole word fits — never cut mid-word; the name stays its title and accessible name
// ABOUTME: Measured against the band's own width, and again when the band is resized

import { useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { clsx } from 'clsx';

interface PhaseBandProps {
  /** The phase's name in the reader's language. */
  label: string;
  className?: string;
  style?: CSSProperties;
  /** Extra room kept clear on the right, so a marker over the band's end never sits on the name. */
  endClear?: boolean;
  'data-testid'?: string;
  'data-kind'?: string;
}

export function PhaseBand({ label, className, style, endClear = false, ...rest }: PhaseBandProps) {
  const outer = useRef<HTMLSpanElement>(null);
  const inner = useRef<HTMLSpanElement>(null);
  const [fits, setFits] = useState(true);

  useLayoutEffect(() => {
    const box = outer.current;
    const text = inner.current;
    if (box === null || text === null) return;
    const measure = () => setFits(text.offsetWidth <= box.clientWidth);
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(box);
    return () => observer.disconnect();
  }, [label]);

  return (
    <span ref={outer} title={label} className={clsx('flex items-center overflow-hidden', className)} style={style} {...rest}>
      <span ref={inner} aria-hidden="true" className={clsx('whitespace-nowrap px-[7px]', endClear && 'pr-4', !fits && 'invisible')}>
        {label}
      </span>
      <span className="sr-only">{label}</span>
    </span>
  );
}
