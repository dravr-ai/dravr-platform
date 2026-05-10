// ABOUTME: Mobile theme context — resolves the active scheme + exposes dynamic palette
// ABOUTME: Wires AsyncStorage preference into NativeWind's color scheme + JS-side colors

import React, { createContext, useContext, useEffect, useMemo } from 'react';
import { View } from 'react-native';
import { useColorScheme } from 'nativewind';
import {
  BOREAL_LIGHT,
  BOREAL_DARK,
  type ColorScheme,
} from '../../../packages/shared-constants/src/design-system';
import { useAppearancePref, type AppearancePref } from '../hooks/useAppearancePref';

// BOREAL_LIGHT / BOREAL_DARK both freeze their values with `as const`, so the
// raw types are mutually exclusive literal-strings. Widen to plain `string`
// so `tokens` can hold either palette without TS complaining at the assign.
type BorealTokens = { readonly [K in keyof typeof BOREAL_LIGHT]: string };

interface ThemeColors {
  /** Boreal MD3 token tree for the active scheme. */
  tokens: BorealTokens;
  /** Pierre legacy palette (icons, indicators, gradient endpoints). */
  pierre: {
    violet: string;
    cyan: string;
    activity: string;
    nutrition: string;
    recovery: string;
    mobility: string;
    red: string;
    dark: string;
    slate: string;
  };
  /** Background tier aliases (canvas / sections / elevated cards). */
  background: {
    primary: string;
    secondary: string;
    tertiary: string;
    elevated: string;
  };
  /** Foreground tier aliases (body / muted / helper / accent). */
  text: {
    primary: string;
    secondary: string;
    tertiary: string;
    accent: string;
  };
  /** Hairline border families (RN style strings include alpha). */
  border: {
    subtle: string;
    default: string;
    strong: string;
  };
  /** Semantic flags. */
  success: string;
  warning: string;
  error: string;
}

interface ThemeContextValue {
  /** User's stored preference: 'system' | 'light' | 'dark'. */
  pref: AppearancePref;
  /** Mutate (and persist) the preference. */
  setPref: (next: AppearancePref) => Promise<void>;
  /** Resolved scheme — never 'system'; what the UI actually renders. */
  scheme: ColorScheme;
  /** Active palette for inline JS styles. */
  colors: ThemeColors;
  /** True while the persisted preference is still loading on cold start. */
  loading: boolean;
}

const PILLAR_LIGHT = {
  activity: '#3c6658',
  nutrition: '#8f6a2e',
  recovery: '#5e7a82',
  mobility: '#7a4d5e',
} as const;

const PILLAR_DARK = {
  activity: '#79a694',
  nutrition: '#d6b87a',
  recovery: '#9bb6bd',
  mobility: '#c4929e',
} as const;

function buildPalette(scheme: ColorScheme): ThemeColors {
  const tokens = scheme === 'dark' ? BOREAL_DARK : BOREAL_LIGHT;
  const pillars = scheme === 'dark' ? PILLAR_DARK : PILLAR_LIGHT;
  const borderRgb = '192, 200, 195';

  return {
    tokens,
    pierre: {
      violet: tokens.primary,
      cyan: tokens.primaryContainer,
      activity: pillars.activity,
      nutrition: pillars.nutrition,
      recovery: pillars.recovery,
      mobility: pillars.mobility,
      red: tokens.error,
      dark: tokens.onSurface,
      slate: tokens.surfaceContainer,
    },
    background: {
      primary: tokens.surface,
      secondary: tokens.surfaceContainerLow,
      tertiary: tokens.surfaceContainer,
      elevated: tokens.surfaceContainerLowest,
    },
    text: {
      primary: tokens.onSurface,
      secondary: tokens.onSurfaceVariant,
      tertiary: tokens.outline,
      accent: tokens.primary,
    },
    border: {
      subtle: `rgba(${borderRgb}, 0.08)`,
      default: `rgba(${borderRgb}, 0.14)`,
      strong: `rgba(${borderRgb}, 0.22)`,
    },
    success: scheme === 'dark' ? '#79a694' : '#2e7d5b',
    warning: scheme === 'dark' ? '#d6b87a' : '#8f6a2e',
    error: tokens.error,
  };
}

// Default context value — used when a component renders outside a
// ThemeProvider (notably in unit tests where wrapping every render in a
// provider would be churn for no signal). The runtime app always wraps in
// ThemeProvider via app/_layout.tsx, so this fallback is purely a test
// convenience: it returns the dark palette plus a noop setter.
const DEFAULT_THEME_VALUE: ThemeContextValue = {
  pref: 'dark',
  setPref: async () => {
    // noop — tests don't persist preferences.
  },
  scheme: 'dark',
  colors: buildPalette('dark'),
  loading: false,
};

const ThemeContext = createContext<ThemeContextValue>(DEFAULT_THEME_VALUE);

interface ThemeProviderProps {
  children: React.ReactNode;
}

export function ThemeProvider({ children }: ThemeProviderProps): React.ReactElement {
  const { pref, loading, setPref } = useAppearancePref();
  const { colorScheme: nwScheme, setColorScheme } = useColorScheme();

  // Resolve user pref -> the scheme the UI renders. 'system' falls through to
  // NativeWind's detected OS scheme, which itself defaults to dark when the
  // OS reports no preference.
  const resolvedScheme: ColorScheme = useMemo(() => {
    if (pref === 'light') return 'light';
    if (pref === 'dark') return 'dark';
    return nwScheme === 'light' ? 'light' : 'dark';
  }, [pref, nwScheme]);

  // Drive NativeWind so Tailwind classes (bg-surface, text-on-surface, etc.)
  // pick up the right CSS-variable bank. Using setColorScheme directly keeps
  // NativeWind in sync with the persisted user preference.
  useEffect(() => {
    if (loading) return;
    if (pref === 'system') {
      setColorScheme('system');
    } else {
      setColorScheme(pref);
    }
  }, [pref, loading, setColorScheme]);

  const colors = useMemo(() => buildPalette(resolvedScheme), [resolvedScheme]);

  const value = useMemo<ThemeContextValue>(
    () => ({
      pref,
      setPref,
      scheme: resolvedScheme,
      colors,
      loading,
    }),
    [pref, setPref, resolvedScheme, colors, loading],
  );

  // NativeWind v4 with `darkMode: 'class'` resolves dark variants when the
  // `dark` class is present on an ancestor. Calling `setColorScheme(pref)` is
  // not enough on its own under RN — wrap the tree in an explicit className
  // wrapper so Tailwind classes (and the CSS variables in global.css) flip
  // correctly on appearance changes.
  return (
    <ThemeContext.Provider value={value}>
      <View className={resolvedScheme === 'dark' ? 'dark' : ''} style={{ flex: 1 }}>
        {children}
      </View>
    </ThemeContext.Provider>
  );
}

/**
 * Hook into the active palette + scheme. Components consuming this re-render
 * automatically when the user toggles appearance from Settings. Returns the
 * dark-default fallback when called outside a ThemeProvider (test renders).
 */
export function useTheme(): ThemeContextValue {
  return useContext(ThemeContext);
}

/**
 * Convenience hook that returns just the palette — matches the shape of the
 * legacy `colors` constant exported from `src/constants/theme.ts`. Components
 * already destructuring `colors.pierre.violet` etc. can swap their import for
 * `const colors = useThemeColors()` with no other changes.
 */
export function useThemeColors(): ThemeColors {
  return useTheme().colors;
}
