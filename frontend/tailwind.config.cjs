// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Boreal Editorial design system — MD3 tokens.
//
// Every token below reads from a CSS custom property defined in src/index.css,
// so a single `.dark` class on <html> flips the entire palette without
// needing `dark:` variants on individual utilities. The legacy `pierre.*`
// namespace it replaced is gone: every value in it was a frozen light-theme
// hex, so each call site was theme-invariant by construction.

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        // ── Boreal MD3 tokens (canonical) ──
        primary: {
          DEFAULT: 'rgb(var(--color-primary) / <alpha-value>)',
          container: 'rgb(var(--color-primary-container) / <alpha-value>)',
          'fixed-dim': 'rgb(var(--color-primary-fixed-dim) / <alpha-value>)',
        },
        'on-primary': 'rgb(var(--color-on-primary) / <alpha-value>)',
        'on-primary-container': 'rgb(var(--color-on-primary-container) / <alpha-value>)',
        'primary-hover': 'rgb(var(--color-primary-hover) / <alpha-value>)',

        tertiary: {
          DEFAULT: 'rgb(var(--color-tertiary) / <alpha-value>)',
          container: 'rgb(var(--color-tertiary-container) / <alpha-value>)',
        },
        'on-tertiary': 'rgb(var(--color-on-tertiary) / <alpha-value>)',
        'on-tertiary-container': 'rgb(var(--color-on-tertiary-container) / <alpha-value>)',

        error: {
          DEFAULT: 'rgb(var(--color-error) / <alpha-value>)',
          container: 'rgb(var(--color-error-container) / <alpha-value>)',
        },
        'on-error': 'rgb(var(--color-on-error) / <alpha-value>)',
        'on-error-container': 'rgb(var(--color-on-error-container) / <alpha-value>)',

        // DESIGN.md §2 feedback palette. success/warning/info existed only as
        // documented hexes until now, which is why call sites reached for
        // text-amber-400 and friends.
        success: 'rgb(var(--color-success) / <alpha-value>)',
        warning: 'rgb(var(--color-warning) / <alpha-value>)',
        info: 'rgb(var(--color-info) / <alpha-value>)',
        // The overlay behind drawers and sheets — a shadow of the page in both themes.
        scrim: 'rgb(var(--color-scrim) / <alpha-value>)',

        // DESIGN.md §2 pillar accents — semantic, so they flip with the theme.
        activity: 'rgb(var(--color-activity) / <alpha-value>)',
        nutrition: 'rgb(var(--color-nutrition) / <alpha-value>)',
        recovery: 'rgb(var(--color-recovery) / <alpha-value>)',
        mobility: 'rgb(var(--color-mobility) / <alpha-value>)',

        // Text that sits ON a tint of the hue above. See index.css.
        'on-activity-container': 'rgb(var(--color-on-activity-container) / <alpha-value>)',
        'on-nutrition-container': 'rgb(var(--color-on-nutrition-container) / <alpha-value>)',
        'on-recovery-container': 'rgb(var(--color-on-recovery-container) / <alpha-value>)',
        'on-mobility-container': 'rgb(var(--color-on-mobility-container) / <alpha-value>)',
        'on-info-container': 'rgb(var(--color-on-info-container) / <alpha-value>)',
        'on-success-container': 'rgb(var(--color-on-success-container) / <alpha-value>)',
        'on-warning-container': 'rgb(var(--color-on-warning-container) / <alpha-value>)',


        surface: {
          DEFAULT: 'rgb(var(--color-surface) / <alpha-value>)',
          dim: 'rgb(var(--color-surface-dim) / <alpha-value>)',
          bright: 'rgb(var(--color-surface-bright) / <alpha-value>)',
          tint: 'rgb(var(--color-surface-tint) / <alpha-value>)',
          variant: 'rgb(var(--color-surface-variant) / <alpha-value>)',
          container: {
            DEFAULT: 'rgb(var(--color-surface-container) / <alpha-value>)',
            lowest: 'rgb(var(--color-surface-container-lowest) / <alpha-value>)',
            low: 'rgb(var(--color-surface-container-low) / <alpha-value>)',
            high: 'rgb(var(--color-surface-container-high) / <alpha-value>)',
            highest: 'rgb(var(--color-surface-container-highest) / <alpha-value>)',
          },
        },
        'on-surface': {
          DEFAULT: 'rgb(var(--color-on-surface) / <alpha-value>)',
          variant: 'rgb(var(--color-on-surface-variant) / <alpha-value>)',
        },

        outline: {
          DEFAULT: 'rgb(var(--color-outline) / <alpha-value>)',
          variant: 'rgb(var(--color-outline-variant) / <alpha-value>)',
        },
        // API tier colors — rebalanced to boreal
        tier: {
          trial: '#8f6a2e',       // warm bronze
          starter: '#3c6658',     // sage
          professional: '#00241a',// primary
          enterprise: '#5e7a82',  // muted slate
        },
        'api-blue': '#234e40',
        'api-green': '#3c6658',
        'api-red': '#ba1a1a',
        'api-yellow': '#8f6a2e',
      },
      fontFamily: {
        // Boreal typography stack per DESIGN.md §3
        sans: ['Plus Jakarta Sans', 'Inter', 'system-ui', '-apple-system', 'Segoe UI', 'Roboto', 'sans-serif'],
        display: ['Space Grotesk', 'Plus Jakarta Sans', 'sans-serif'],
        headline: ['Space Grotesk', 'Plus Jakarta Sans', 'sans-serif'],
        label: ['Inter', 'system-ui', 'sans-serif'],
        mono: ['JetBrains Mono', 'Monaco', 'Menlo', 'Ubuntu Mono', 'Consolas', 'monospace'],
      },
      fontSize: {
        'xs': '0.75rem',
        'sm': '0.875rem',
        'base': '1rem',
        'lg': '1.125rem',
        'xl': '1.25rem',
        '2xl': '1.5rem',
        '3xl': '1.875rem',
        '4xl': '2.25rem',
        // Mobile-first responsive scale — use with `text-h1-mobile md:text-3xl`
        // to step up at the md breakpoint. See ADR / plan
        // "Web Frontend Mobile-Friendly Redesign".
        'h1-mobile': ['1.5rem', { lineHeight: '1.2', fontWeight: '600' }],
        'h2-mobile': ['1.25rem', { lineHeight: '1.3', fontWeight: '600' }],
        'h3-mobile': ['1.125rem', { lineHeight: '1.35', fontWeight: '600' }],
        'body-mobile': ['0.9375rem', { lineHeight: '1.5' }],
      },
      letterSpacing: {
        brand: '0.15em',   // DRAVR wordmark
        label: '0.05em',   // tertiary buttons / small caps
      },
      spacing: {
        '1': '0.25rem',
        '2': '0.5rem',
        '3': '0.75rem',
        '4': '1rem',
        '5': '1.25rem',
        '6': '1.5rem',
        '8': '2rem',
        '10': '2.5rem',
        '12': '3rem',
        '16': '4rem',
        '20': '5rem',
        '24': '6rem',
        'section': '8.5rem', // DESIGN.md §6 "24" spacing for major breaks
      },
      borderRadius: {
        // Boreal scale — tight geometric precision (DESIGN.md §6)
        'sm': '0.125rem',
        'md': '0.25rem',
        'lg': '0.5rem',
        'xl': '0.75rem',
        // `full` (9999px) still available from core for chips only
      },
      boxShadow: {
        // Boreal Product Tier elevation stack — two-layer shadows. Reads
        // from CSS vars so dark mode swaps to the deepened recipe via
        // `html.dark` overrides in index.css.
        'card': 'var(--shadow-card)',
        'card-hover': 'var(--shadow-card-hover)',
        'floating': 'var(--shadow-floating)',
        'ambient': 'var(--shadow-card)',
        // Tailwind sm/md/lg/xl utility scale, tuned to the same two-layer recipe.
        'sm': '0 1px 2px rgba(26, 28, 27, 0.05)',
        'md': '0 4px 8px -2px rgba(26, 28, 27, 0.08), 0 2px 4px -1px rgba(26, 28, 27, 0.06)',
        'lg': '0 12px 24px -6px rgba(26, 28, 27, 0.12), 0 6px 12px -3px rgba(26, 28, 27, 0.08)',
        'xl': '0 24px 48px -12px rgba(26, 28, 27, 0.18), 0 8px 16px -4px rgba(26, 28, 27, 0.10)',
        // Legacy glow-* names — kept resolving as the resting card shadow so
        // call sites compile during the sweep.
        'glow': 'var(--shadow-card)',
        'glow-sm': 'var(--shadow-card)',
        'glow-lg': 'var(--shadow-card-hover)',
        'glow-violet': 'var(--shadow-card)',
        'glow-cyan': 'var(--shadow-card)',
        'glow-activity': 'var(--shadow-card)',
        'glow-nutrition': 'var(--shadow-card)',
        'glow-recovery': 'var(--shadow-card)',
        // Inset ghost border, repointed at the lifted Product Tier opacity.
        'glass': 'inset 0 0 0 1px rgba(155, 165, 159, 0.40)',
      },
      // Coach prose runs through @tailwindcss/typography, whose stock palette
      // is Tailwind gray in both schemes. Every prose variable — and its
      // `invert` twin, so `dark:prose-invert` stays inert — reads the Boreal
      // tokens instead, which already flip with the theme.
      typography: {
        DEFAULT: {
          css: {
            '--tw-prose-body': 'rgb(var(--color-on-surface))',
            '--tw-prose-headings': 'rgb(var(--color-on-surface))',
            '--tw-prose-lead': 'rgb(var(--color-on-surface-variant))',
            '--tw-prose-links': 'rgb(var(--color-primary))',
            '--tw-prose-bold': 'rgb(var(--color-on-surface))',
            '--tw-prose-counters': 'rgb(var(--color-on-surface-variant))',
            '--tw-prose-bullets': 'rgb(var(--color-outline-variant))',
            '--tw-prose-hr': 'var(--ghost-border)',
            '--tw-prose-quotes': 'rgb(var(--color-on-surface))',
            '--tw-prose-quote-borders': 'rgb(var(--color-primary))',
            '--tw-prose-captions': 'rgb(var(--color-on-surface-variant))',
            '--tw-prose-code': 'rgb(var(--color-on-surface))',
            '--tw-prose-pre-code': 'rgb(var(--color-on-surface))',
            '--tw-prose-pre-bg': 'rgb(var(--color-surface-container-high))',
            '--tw-prose-th-borders': 'var(--ghost-border-strong)',
            '--tw-prose-td-borders': 'var(--ghost-border)',
            '--tw-prose-invert-body': 'rgb(var(--color-on-surface))',
            '--tw-prose-invert-headings': 'rgb(var(--color-on-surface))',
            '--tw-prose-invert-lead': 'rgb(var(--color-on-surface-variant))',
            '--tw-prose-invert-links': 'rgb(var(--color-primary))',
            '--tw-prose-invert-bold': 'rgb(var(--color-on-surface))',
            '--tw-prose-invert-counters': 'rgb(var(--color-on-surface-variant))',
            '--tw-prose-invert-bullets': 'rgb(var(--color-outline-variant))',
            '--tw-prose-invert-hr': 'var(--ghost-border)',
            '--tw-prose-invert-quotes': 'rgb(var(--color-on-surface))',
            '--tw-prose-invert-quote-borders': 'rgb(var(--color-primary))',
            '--tw-prose-invert-captions': 'rgb(var(--color-on-surface-variant))',
            '--tw-prose-invert-code': 'rgb(var(--color-on-surface))',
            '--tw-prose-invert-pre-code': 'rgb(var(--color-on-surface))',
            '--tw-prose-invert-pre-bg': 'rgb(var(--color-surface-container-high))',
            '--tw-prose-invert-th-borders': 'var(--ghost-border-strong)',
            '--tw-prose-invert-td-borders': 'var(--ghost-border)',
          },
        },
      },
      backdropBlur: {
        xs: '2px',
        boreal: '12px', // canonical glass blur (DESIGN.md §2)
      },
      transitionDuration: {
        'fast': '150ms',
        'base': '200ms',
        'slow': '300ms',
      },
      backgroundImage: {
        // Canonical 145° primary → primary_container
        'boreal-hero': 'linear-gradient(145deg, rgb(var(--color-primary)) 0%, rgb(var(--color-primary-container)) 100%)',
        // Legacy gradient names — all repoint at the boreal hero so content
        // sweep can delete or rename them safely.
        'gradient-activity': 'linear-gradient(145deg, #3c6658 0%, #234e40 100%)',
        'gradient-nutrition': 'linear-gradient(145deg, #8f6a2e 0%, #6e5020 100%)',
        'gradient-recovery': 'linear-gradient(145deg, #5e7a82 0%, #425962 100%)',
        'gradient-mobility': 'linear-gradient(145deg, #7a4d5e 0%, #5a3744 100%)',
      },
      animation: {
        'fade-rise': 'fadeRise 500ms cubic-bezier(0.22, 1, 0.36, 1) both',
        'fade-in': 'fadeIn 0.15s ease-out',
        'slide-up': 'slideUp 0.2s ease-out',
        'scale-in': 'scaleIn 0.2s ease-out',
      },
      keyframes: {
        fadeRise: {
          '0%': { opacity: '0', transform: 'translateY(8px)' },
          '100%': { opacity: '1', transform: 'translateY(0)' },
        },
        fadeIn: {
          '0%': { opacity: '0' },
          '100%': { opacity: '1' },
        },
        slideUp: {
          '0%': { transform: 'translateY(10px)', opacity: '0' },
          '100%': { transform: 'translateY(0)', opacity: '1' },
        },
        scaleIn: {
          '0%': { transform: 'scale(0.95)', opacity: '0' },
          '100%': { transform: 'scale(1)', opacity: '1' },
        },
      },
    },
  },
  plugins: [
    require('@tailwindcss/forms'),
    require('@tailwindcss/typography'),
  ],
}
