// ABOUTME: NativeWind v4 Tailwind config — Dravr Boreal Editorial tokens
// ABOUTME: Matches frontend/tailwind.config.cjs and packages/shared-constants/src/design-system.ts

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./App.{js,jsx,ts,tsx}",
    "./app/**/*.{js,jsx,ts,tsx}",
    "./src/**/*.{js,jsx,ts,tsx}",
  ],
  presets: [require('nativewind/preset')],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        // ── Boreal MD3 canonical tokens (light) ──
        primary: {
          DEFAULT: '#00241a',
          container: '#0d3b2e',
          fixed: '#beedd9',
          'fixed-dim': '#a3d0be',
        },
        'on-primary': '#ffffff',
        'on-primary-container': '#79a694',

        tertiary: {
          DEFAULT: '#03231d',
          container: '#1b3932',
          'fixed-dim': '#adcdc3',
        },
        'on-tertiary': '#ffffff',
        'on-tertiary-container': '#83a399',

        error: {
          DEFAULT: '#ba1a1a',
          container: '#ffdad6',
        },
        'on-error': '#ffffff',
        'on-error-container': '#93000a',

        surface: {
          DEFAULT: '#f9f9f6',
          dim: '#dadad7',
          bright: '#f9f9f6',
          tint: '#3c6658',
          variant: '#e2e3e0',
          container: {
            DEFAULT: '#eeeeeb',
            lowest: '#ffffff',
            low: '#f4f4f1',
            high: '#e8e8e5',
            highest: '#e2e3e0',
          },
        },
        'on-surface': {
          DEFAULT: '#1a1c1b',
          variant: '#414845',
        },
        outline: {
          DEFAULT: '#717974',
          variant: '#c0c8c3',
        },

        // ── Legacy `pierre.*` namespace — repointed at Boreal semantics ──
        // Keeps in-tree bg-pierre-* class references rendering in boreal tones
        // during the sweep phase. Migrate call sites over time, then drop.
        pierre: {
          violet: '#00241a',
          cyan: '#0d3b2e',
          activity: '#3c6658',
          nutrition: '#8f6a2e',
          recovery: '#5e7a82',
          mobility: '#7a4d5e',
          red: '#ba1a1a',
          dark: '#1a1c1b',
          slate: '#eeeeeb',
          gray: {
            50: '#f9f9f6',
            100: '#f4f4f1',
            200: '#eeeeeb',
            300: '#e8e8e5',
            400: '#c0c8c3',
            500: '#717974',
            600: '#414845',
            700: '#2f312f',
            800: '#1a1c1b',
            900: '#11130f',
          },
          green: {
            50: '#eef4f1',
            100: '#d6e3dc',
            500: '#3c6658',
            600: '#234e40',
            700: '#0d3b2e',
          },
          yellow: {
            50: '#f5efdf',
            100: '#e9dcb0',
            500: '#8f6a2e',
            600: '#6e5020',
            700: '#4c3716',
          },
          blue: {
            50: '#eef4f1',
            100: '#d6e3dc',
            500: '#3c6658',
            600: '#234e40',
            700: '#0d3b2e',
          },
        },

        // Legacy aliases still used in theme.ts + app components.
        primary_scale: {
          50: '#eef4f1',
          100: '#d6e3dc',
          200: '#a3d0be',
          300: '#79a694',
          400: '#5e8a78',
          500: '#3c6658',
          600: '#234e40',
          700: '#0d3b2e',
          800: '#002117',
          900: '#00241a',
          950: '#001812',
        },

        background: {
          primary: '#f9f9f6',           // surface — base canvas
          secondary: '#f4f4f1',         // surface-container-low — sections
          tertiary: '#eeeeeb',          // surface-container — elevated
          elevated: '#ffffff',          // surface-container-lowest — floating cards
        },

        text: {
          primary: '#1a1c1b',           // on-surface — body copy
          secondary: '#414845',         // on-surface-variant
          tertiary: '#717974',          // outline — helper / label
          accent: '#00241a',            // primary — links, active state
        },

        border: {
          subtle: 'rgba(192, 200, 195, 0.10)',
          DEFAULT: 'rgba(192, 200, 195, 0.15)',  // ghost border baseline
          strong: 'rgba(192, 200, 195, 0.25)',
        },

        success: '#2e7d5b',
        warning: '#8f6a2e',

        // Provider brand colors — belong to third parties, unchanged
        providers: {
          strava: '#FC4C02',
          garmin: '#007CC3',
          fitbit: '#00B0B9',
          whoop: '#00D46A',
          terra: '#6366F1',
        },
      },
      fontFamily: {
        // Space Grotesk for display/headlines, Plus Jakarta Sans for body,
        // Inter for labels, Newsreader for editorial serif accents.
        // Loaded at runtime via expo-font in app/_layout.tsx.
        display: ['SpaceGrotesk', 'System', 'sans-serif'],
        headline: ['SpaceGrotesk', 'System', 'sans-serif'],
        sans: ['PlusJakartaSans', 'System', 'sans-serif'],
        label: ['Inter', 'System', 'sans-serif'],
        serif: ['Newsreader', 'Georgia', 'serif'],
        mono: ['JetBrainsMono', 'Menlo', 'monospace'],
      },
      letterSpacing: {
        brand: '0.15em',      // DRAVR wordmark
        label: '0.05em',      // small-caps buttons
      },
      borderRadius: {
        none: '0',
        sm: '2px',
        DEFAULT: '4px',
        md: '4px',
        lg: '8px',
        xl: '12px',
        '2xl': '12px',
        full: '9999px',
      },
      boxShadow: {
        // Ambient boreal shadow (6% on_surface, blur 24, spread -4).
        // RN note: most shadows are applied via style props, not NativeWind
        // classes; keep the utilities available for compatibility with web
        // components that still reference them during the sweep.
        ambient: '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        card: '0 24px 48px -12px rgba(26, 28, 27, 0.06)',
        'glow-violet': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-cyan': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-activity': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-nutrition': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
        'glow-recovery': '0 24px 48px -4px rgba(26, 28, 27, 0.06)',
      },
    },
  },
  plugins: [],
};
