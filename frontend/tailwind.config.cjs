// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Boreal Editorial design system — MD3 tokens.
//
// Every token below reads from a CSS custom property defined in src/index.css,
// so a single `.dark` class on <html> flips the entire palette without
// needing `dark:` variants on individual utilities. The legacy `pierre.*`
// namespace is kept as a repointed alias during the content sweep.

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
          DEFAULT: 'var(--color-primary)',
          container: 'var(--color-primary-container)',
          'fixed-dim': 'var(--color-primary-fixed-dim)',
        },
        'on-primary': 'var(--color-on-primary)',
        'on-primary-container': 'var(--color-on-primary-container)',

        tertiary: {
          DEFAULT: 'var(--color-tertiary)',
          container: 'var(--color-tertiary-container)',
        },
        'on-tertiary': 'var(--color-on-tertiary)',
        'on-tertiary-container': 'var(--color-on-tertiary-container)',

        error: {
          DEFAULT: 'var(--color-error)',
          container: 'var(--color-error-container)',
        },
        'on-error': 'var(--color-on-error)',
        'on-error-container': 'var(--color-on-error-container)',

        surface: {
          DEFAULT: 'var(--color-surface)',
          dim: 'var(--color-surface-dim)',
          bright: 'var(--color-surface-bright)',
          tint: 'var(--color-surface-tint)',
          variant: 'var(--color-surface-variant)',
          container: {
            DEFAULT: 'var(--color-surface-container)',
            lowest: 'var(--color-surface-container-lowest)',
            low: 'var(--color-surface-container-low)',
            high: 'var(--color-surface-container-high)',
            highest: 'var(--color-surface-container-highest)',
          },
        },
        'on-surface': {
          DEFAULT: 'var(--color-on-surface)',
          variant: 'var(--color-on-surface-variant)',
        },

        outline: {
          DEFAULT: 'var(--color-outline)',
          variant: 'var(--color-outline-variant)',
        },

        // ── Legacy `pierre.*` namespace — repointed at Boreal semantics ──
        // Kept during the rebrand so ~140 in-tree references keep resolving.
        // Migrate call sites to canonical MD3 names (primary, surface, outline…)
        // and this block can be deleted.
        pierre: {
          violet: '#00241a',           // → primary
          cyan: '#0d3b2e',              // → primary-container
          activity: '#3c6658',          // pillar sage
          nutrition: '#8f6a2e',         // pillar warm bronze
          recovery: '#5e7a82',          // pillar muted slate
          mobility: '#7a4d5e',          // pillar aged rose
          dark: '#1a1c1b',              // → on-surface (ink, not bg)
          slate: '#eeeeeb',             // → surface-container
          'violet-light': '#234e40',    // on_primary_fixed_variant
          'violet-dark': '#002117',     // on_primary_fixed
          'cyan-light': '#a3d0be',      // primary_fixed_dim
          'cyan-dark': '#0d3b2e',
          'activity-light': '#5e8a78',
          'activity-dark': '#234e40',
          'nutrition-light': '#b98a47',
          'nutrition-dark': '#6e5020',
          'recovery-light': '#839ba2',
          'recovery-dark': '#425962',
          'mobility-light': '#9a6b7b',
          'mobility-dark': '#5a3744',
          // Grayscale legacy scales mapped onto the Boreal neutral ramp.
          blue: {
            50: '#eef4f1', 100: '#d6e3dc', 200: '#a3d0be', 300: '#79a694',
            400: '#5e8a78', 500: '#3c6658', 600: '#234e40', 700: '#0d3b2e',
            800: '#002117', 900: '#00241a',
          },
          gray: {
            50: '#f9f9f6', 100: '#f4f4f1', 200: '#eeeeeb', 300: '#e8e8e5',
            400: '#c0c8c3', 500: '#717974', 600: '#414845', 700: '#2f312f',
            800: '#1a1c1b', 900: '#11130f',
          },
          green: {
            50: '#eef4f1', 100: '#d6e3dc', 500: '#3c6658',
            600: '#234e40', 700: '#0d3b2e', 800: '#002117',
          },
          yellow: {
            50: '#f5efdf', 100: '#e9dcb0', 500: '#8f6a2e',
            600: '#6e5020', 700: '#4c3716', 800: '#2a1e0c',
          },
          red: {
            50: '#ffdad6', 100: '#ffb4ab', 500: '#ba1a1a',
            600: '#93000a', 700: '#690005', 800: '#410002',
          },
          purple: {
            50: '#f0eaee', 100: '#d5c5cd', 500: '#7a4d5e',
            600: '#5a3744', 700: '#3b222c', 800: '#1d0e14',
          },
          teal: {
            50: '#eef4f1', 100: '#c8dcd3', 200: '#a3d0be', 300: '#79a694',
            400: '#5e8a78', 500: '#3c6658', 600: '#234e40', 700: '#0d3b2e',
            800: '#002117',
          },
          pink: {
            50: '#f0eaee', 100: '#d5c5cd', 200: '#b89daa', 500: '#7a4d5e',
            600: '#5a3744', 700: '#3b222c', 800: '#1d0e14',
          },
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
        serif: ['Newsreader', 'Georgia', 'serif'],
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
        // Boreal ambient shadows — 6% on_surface, 24px blur, -4px spread.
        // No glow presets. Elevation comes from tonal layering first.
        'ambient': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'card': '0 24px 48px -12px rgba(26, 28, 27, 0.06)',
        // Retain sm/md/lg/xl as softer variants so legacy classes don't blow up.
        'sm': '0 1px 2px 0 rgba(26, 28, 27, 0.04)',
        'md': '0 8px 16px -4px rgba(26, 28, 27, 0.05)',
        'lg': '0 16px 32px -4px rgba(26, 28, 27, 0.06)',
        'xl': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        // Legacy glow-* names repointed at ambient shadow so old classes render
        // quietly instead of crashing during the content sweep.
        'glow': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-sm': '0 16px 32px -4px rgba(26, 28, 27, 0.05)',
        'glow-lg': '0 32px 64px -8px rgba(26, 28, 27, 0.08)',
        'glow-violet': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-cyan': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-activity': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-nutrition': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-recovery': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        // Ghost border fallback — 1px inset ghost border for accessibility
        'glass': 'inset 0 0 0 1px rgba(192, 200, 195, 0.15)',
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
        'boreal-hero': 'linear-gradient(145deg, #00241a 0%, #0d3b2e 100%)',
        // Legacy gradient names — all repoint at the boreal hero so content
        // sweep can delete or rename them safely.
        'gradient-pierre': 'linear-gradient(145deg, #00241a 0%, #0d3b2e 100%)',
        'gradient-pierre-horizontal': 'linear-gradient(90deg, #00241a 0%, #0d3b2e 100%)',
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
