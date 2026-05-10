// ABOUTME: NativeWind v4 Tailwind config — Dravr Boreal Editorial tokens
// ABOUTME: CSS variables drive light/dark — values declared in global.css

/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    "./App.{js,jsx,ts,tsx}",
    "./app/**/*.{js,jsx,ts,tsx}",
    "./src/**/*.{js,jsx,ts,tsx}",
  ],
  presets: [require('nativewind/preset')],
  // The `dark` class is toggled on the root view by the ThemeProvider; values
  // for every `--color-*` token below live in global.css. Light is the
  // declarative default, dark overrides under the `.dark` selector.
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        // ── Boreal MD3 canonical tokens ──
        primary: {
          DEFAULT: 'rgb(var(--color-primary) / <alpha-value>)',
          container: 'rgb(var(--color-primary-container) / <alpha-value>)',
          fixed: 'rgb(var(--color-primary-fixed) / <alpha-value>)',
          'fixed-dim': 'rgb(var(--color-primary-fixed-dim) / <alpha-value>)',
        },
        'on-primary': 'rgb(var(--color-on-primary) / <alpha-value>)',
        'on-primary-container': 'rgb(var(--color-on-primary-container) / <alpha-value>)',

        tertiary: {
          DEFAULT: 'rgb(var(--color-tertiary) / <alpha-value>)',
          container: 'rgb(var(--color-tertiary-container) / <alpha-value>)',
          'fixed-dim': 'rgb(var(--color-tertiary-fixed-dim) / <alpha-value>)',
        },
        'on-tertiary': 'rgb(var(--color-on-tertiary) / <alpha-value>)',
        'on-tertiary-container': 'rgb(var(--color-on-tertiary-container) / <alpha-value>)',

        error: {
          DEFAULT: 'rgb(var(--color-error) / <alpha-value>)',
          container: 'rgb(var(--color-error-container) / <alpha-value>)',
        },
        'on-error': 'rgb(var(--color-on-error) / <alpha-value>)',
        'on-error-container': 'rgb(var(--color-on-error-container) / <alpha-value>)',

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

        // ── Legacy `pierre.*` namespace — repointed at Boreal CSS variables ──
        // Keeps in-tree bg-pierre-* class references valid during the sweep.
        // Pillar tints harmonize on both canvases via the --color-pillar-* vars.
        pierre: {
          violet: 'rgb(var(--color-primary) / <alpha-value>)',
          cyan: 'rgb(var(--color-primary-container) / <alpha-value>)',
          activity: 'rgb(var(--color-pillar-activity) / <alpha-value>)',
          nutrition: 'rgb(var(--color-pillar-nutrition) / <alpha-value>)',
          recovery: 'rgb(var(--color-pillar-recovery) / <alpha-value>)',
          mobility: 'rgb(var(--color-pillar-mobility) / <alpha-value>)',
          red: 'rgb(var(--color-error) / <alpha-value>)',
          dark: 'rgb(var(--color-on-surface) / <alpha-value>)',
          slate: 'rgb(var(--color-surface-container) / <alpha-value>)',
          gray: {
            50: 'rgb(var(--color-gray-50) / <alpha-value>)',
            100: 'rgb(var(--color-gray-100) / <alpha-value>)',
            200: 'rgb(var(--color-gray-200) / <alpha-value>)',
            300: 'rgb(var(--color-gray-300) / <alpha-value>)',
            400: 'rgb(var(--color-gray-400) / <alpha-value>)',
            500: 'rgb(var(--color-gray-500) / <alpha-value>)',
            600: 'rgb(var(--color-gray-600) / <alpha-value>)',
            700: 'rgb(var(--color-gray-700) / <alpha-value>)',
            800: 'rgb(var(--color-gray-800) / <alpha-value>)',
            900: 'rgb(var(--color-gray-900) / <alpha-value>)',
          },
          green: {
            50: 'rgb(var(--color-green-50) / <alpha-value>)',
            100: 'rgb(var(--color-green-100) / <alpha-value>)',
            500: 'rgb(var(--color-green-500) / <alpha-value>)',
            600: 'rgb(var(--color-green-600) / <alpha-value>)',
            700: 'rgb(var(--color-green-700) / <alpha-value>)',
          },
          yellow: {
            50: 'rgb(var(--color-yellow-50) / <alpha-value>)',
            100: 'rgb(var(--color-yellow-100) / <alpha-value>)',
            500: 'rgb(var(--color-yellow-500) / <alpha-value>)',
            600: 'rgb(var(--color-yellow-600) / <alpha-value>)',
            700: 'rgb(var(--color-yellow-700) / <alpha-value>)',
          },
          blue: {
            50: 'rgb(var(--color-blue-50) / <alpha-value>)',
            100: 'rgb(var(--color-blue-100) / <alpha-value>)',
            500: 'rgb(var(--color-blue-500) / <alpha-value>)',
            600: 'rgb(var(--color-blue-600) / <alpha-value>)',
            700: 'rgb(var(--color-blue-700) / <alpha-value>)',
          },
        },

        // Legacy primary_scale — full ladder kept stable across both canvases
        // (used in JS-side gradient endpoints that don't flip).
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
          primary: 'rgb(var(--color-surface) / <alpha-value>)',
          secondary: 'rgb(var(--color-surface-container-low) / <alpha-value>)',
          tertiary: 'rgb(var(--color-surface-container) / <alpha-value>)',
          elevated: 'rgb(var(--color-surface-container-lowest) / <alpha-value>)',
        },

        text: {
          primary: 'rgb(var(--color-on-surface) / <alpha-value>)',
          secondary: 'rgb(var(--color-on-surface-variant) / <alpha-value>)',
          tertiary: 'rgb(var(--color-outline) / <alpha-value>)',
          accent: 'rgb(var(--color-primary) / <alpha-value>)',
        },

        border: {
          subtle: 'rgb(var(--color-border) / 0.08)',
          DEFAULT: 'rgb(var(--color-border) / 0.14)',
          strong: 'rgb(var(--color-border) / 0.22)',
        },

        success: 'rgb(var(--color-success) / <alpha-value>)',
        warning: 'rgb(var(--color-warning) / <alpha-value>)',

        // Provider brand colors — belong to third parties, unchanged across modes.
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
        // Static shadow recipes — deepened for the dark canvas. RN style props
        // get the fully tuned values from the JS theme tokens; these utilities
        // exist for compatibility with components that still reference the
        // class form during the sweep.
        ambient: '0 24px 48px -4px rgba(0, 0, 0, 0.45)',
        card: '0 24px 48px -12px rgba(0, 0, 0, 0.55)',
        'glow-violet': '0 24px 48px -4px rgba(0, 0, 0, 0.45)',
        'glow-cyan': '0 24px 48px -4px rgba(0, 0, 0, 0.45)',
        'glow-activity': '0 24px 48px -4px rgba(0, 0, 0, 0.45)',
        'glow-nutrition': '0 24px 48px -4px rgba(0, 0, 0, 0.45)',
        'glow-recovery': '0 24px 48px -4px rgba(0, 0, 0, 0.45)',
      },
    },
  },
  plugins: [],
};
