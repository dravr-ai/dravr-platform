// ABOUTME: NativeWind v4 Tailwind config — Dravr Boreal v2.2 tokens for the phone
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

        // The one veil behind a sheet or a dialog: `bg-scrim/60`, the web's
        // value. Near-ink in light, black in dark (global.css).
        scrim: 'rgb(var(--color-scrim) / <alpha-value>)',

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

        // Hairlines, the web's three strengths (frontend/src/index.css
        // `--ghost-border*`): `faint` for the dividers inside a list, the
        // default for pane edges, fields and the composer, `strong` for a
        // control's outline. Each variable carries its own alpha per scheme,
        // because a pale line needs more opacity on near-black than a dark
        // line needs on paper; the runtime twin is `useThemeColors().border`.
        border: {
          faint: 'var(--ghost-border-faint)',
          DEFAULT: 'var(--ghost-border)',
          strong: 'var(--ghost-border-strong)',
        },

        success: 'rgb(var(--color-success) / <alpha-value>)',
        warning: 'rgb(var(--color-warning) / <alpha-value>)',
        info: 'rgb(var(--color-info) / <alpha-value>)',

        // DESIGN.md §2 pillar accents — semantic, so they flip with the theme.
        activity: 'rgb(var(--color-activity) / <alpha-value>)',
        nutrition: 'rgb(var(--color-nutrition) / <alpha-value>)',
        recovery: 'rgb(var(--color-recovery) / <alpha-value>)',
        mobility: 'rgb(var(--color-mobility) / <alpha-value>)',

        // Text that sits ON a tint of the hue above — the bound ink each hue
        // carries. At the /15 tint the hue drawn as its own label measures
        // 1.95:1 at worst in light and the ink 4.58:1; DESIGN.md §2 "Bound
        // ink" carries the envelope. Values in global.css, the same names and
        // the same triples the web config maps. The runtime path is
        // `useThemeColors().ink.*`; these are the class path.
        'on-activity-container': 'rgb(var(--color-on-activity-container) / <alpha-value>)',
        'on-nutrition-container': 'rgb(var(--color-on-nutrition-container) / <alpha-value>)',
        'on-recovery-container': 'rgb(var(--color-on-recovery-container) / <alpha-value>)',
        'on-mobility-container': 'rgb(var(--color-on-mobility-container) / <alpha-value>)',
        'on-info-container': 'rgb(var(--color-on-info-container) / <alpha-value>)',
        'on-success-container': 'rgb(var(--color-on-success-container) / <alpha-value>)',
        'on-warning-container': 'rgb(var(--color-on-warning-container) / <alpha-value>)',

        // Provider brand colors — belong to third parties, unchanged across modes.
        providers: {
          strava: '#FC4C02',
          garmin: '#007CC3',
          fitbit: '#00B0B9',
          whoop: '#00D46A',
          terra: '#6366F1',
        },
      },
      // Two faces beside the system one, loaded in app/_layout.tsx under these
      // family names: Schibsted Grotesk 600 for the wordmark and the auth and
      // onboarding headline, JetBrains Mono for counts, times and code. Every
      // other string renders in the platform face (SF, Roboto), which is what
      // Dynamic Type and the native chrome already use.
      fontFamily: {
        display: ['SchibstedGrotesk'],
        mono: ['JetBrainsMono'],
      },
      // The platform ladder in the system face (Design/Boreal v2.2 — Mobile
      // Less, "The scale"). Every step carries its line-height so a bare
      // `text-sm` is complete without a `leading-*` beside it. Reading text is
      // `base` 16/22; interface text is `sm` 13/18 at weight 500 on anything
      // navigable; `xs` 12/16 is the floor — nothing goes under it but a native
      // badge. `2xl` and up are reserved for the auth and onboarding headline.
      fontSize: {
        xs: ['12px', { lineHeight: '16px' }],
        sm: ['13px', { lineHeight: '18px' }],
        base: ['16px', { lineHeight: '22px' }],
        lg: ['17px', { lineHeight: '22px' }],
        xl: ['20px', { lineHeight: '25px' }],
        '2xl': ['22px', { lineHeight: '28px' }],
        '3xl': ['26px', { lineHeight: '32px' }],
      },
      letterSpacing: {
        brand: '0.15em',      // DRAVR wordmark — the only tracked text in the product
      },
      // 4 chips · 8 buttons and fields · 12 floating cards · 20 a sheet's top
      // and the composer field · full for avatars and badges. `2xl` is pinned
      // to `xl` so nothing between a card and a sheet exists.
      borderRadius: {
        none: '0',
        sm: '2px',
        DEFAULT: '4px',
        md: '4px',
        lg: '8px',
        xl: '12px',
        '2xl': '12px',
        '3xl': '20px',
        full: '9999px',
      },
      // The only shadow in the system: what floats over the page. A resting
      // card is lifted by its hairline (DESIGN.md §4).
      boxShadow: {
        floating: '0 12px 24px rgba(0, 0, 0, 0.55)',
      },
    },
  },
  plugins: [],
};
