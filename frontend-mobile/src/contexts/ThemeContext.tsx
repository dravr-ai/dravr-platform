// ABOUTME: Mobile theme context — resolves the active scheme + exposes dynamic palette
// ABOUTME: Wires AsyncStorage preference into NativeWind's color scheme + JS-side colors

import React, { createContext, useCallback, useContext, useEffect, useMemo } from 'react';
import { View } from 'react-native';
import { useColorScheme } from 'nativewind';
import Toast from 'react-native-toast-message';
import { useTranslation } from '@pierre/i18n';
import type { ThemePreference } from '@pierre/shared-types';
import { userApi } from '../services/api';
import {
  BOREAL_LIGHT,
  BOREAL_DARK,
  PILLARS,
  SEMANTIC_COLORS,
  SEMANTIC_COLORS_DARK,
  type ColorScheme,
} from '../../../packages/shared-constants/src/design-system';
import { useAppearancePref, type AppearancePref } from '../hooks/useAppearancePref';

// BOREAL_LIGHT / BOREAL_DARK both freeze their values with `as const`, so the
// raw types are mutually exclusive literal-strings. Widen to plain `string`
// so `tokens` can hold either palette without TS complaining at the assign.
type BorealTokens = { readonly [K in keyof typeof BOREAL_LIGHT]: string };

export interface ThemeColors {
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
    /** Links and active state — the brand ink, legible in both schemes. */
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
  /** Informational tint (DESIGN.md §2). */
  info: string;
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

/**
 * Ghost-border recipes, per scheme (DESIGN.md §2 "Outline / borders").
 *
 * A hairline only reads when it contrasts with what it sits on, and the two
 * schemes need opposite ink for that: a pale grey-green line carries on the
 * near-black canvas and disappears on a white card, which is how the light
 * theme lost the only separation its near-identical surface tiers had. Light
 * therefore takes the darker Product Tier ghost border; dark keeps the pale
 * one at the lower opacities a near-black ground needs.
 */
const BORDER_INK: Record<ColorScheme, { rgb: string; subtle: number; default: number; strong: number }> = {
  light: { rgb: '155, 165, 159', subtle: 0.22, default: 0.4, strong: 0.55 },
  dark: { rgb: '192, 200, 195', subtle: 0.08, default: 0.14, strong: 0.22 },
};

function buildPalette(scheme: ColorScheme): ThemeColors {
  const tokens = scheme === 'dark' ? BOREAL_DARK : BOREAL_LIGHT;
  const pillars = PILLARS[scheme];
  const ink = BORDER_INK[scheme];

  return {
    tokens,
    pierre: {
      violet: tokens.primary,
      // The gradient endpoint beside `violet`. Boreal v2 made `primaryContainer`
      // a pale tint (the athlete bubble's ground), so a violet→container
      // gradient would fade to paper in light; `onPrimaryContainer` is the same
      // hue carried the other way — forest in light, pale mint in dark — which
      // keeps the settings header's sweep a green-to-green one in both schemes.
      cyan: tokens.onPrimaryContainer,
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
      subtle: `rgba(${ink.rgb}, ${ink.subtle})`,
      default: `rgba(${ink.rgb}, ${ink.default})`,
      strong: `rgba(${ink.rgb}, ${ink.strong})`,
    },
    // Read the shared feedback set rather than restating it. These were four
    // hardcoded ternaries, and one of them had drifted: light `warning` was
    // `#8f6a2e`, the EDITORIAL tier value, while global.css and
    // shared-constants both carried the Product tier `#b08326`. The phone
    // therefore answered with two different ambers for one token depending on
    // whether a component read a NativeWind class or this hook — which is the
    // same drift DESIGN.md §2 records mobile carrying for months, fixed in the
    // other two mirrors and missed here.
    ...(scheme === 'dark' ? SEMANTIC_COLORS_DARK : SEMANTIC_COLORS),
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
  const { pref, loading, setPref: persistPref } = useAppearancePref();
  const { colorScheme: nwScheme, setColorScheme } = useColorScheme();
  const { t } = useTranslation();

  // Both theme controls (the header sun/moon toggle and Settings → Appearance)
  // land here, so this is the one place the choice reaches the server.
  // `system` is stored as `null` — no pin, follow the device. The write is
  // fire-and-forget: the local flip has already been persisted and rendered,
  // and a failed request only surfaces as an error toast, never a revert.
  const setPref = useCallback(
    async (next: AppearancePref) => {
      await persistPref(next);
      const theme: ThemePreference = next === 'system' ? null : next;
      userApi.updateTheme(theme).catch(() => {
        Toast.show({
          type: 'error',
          text1: t('settings.theme'),
          text2: t('settings.themeSyncFailed'),
        });
      });
    },
    [persistPref, t],
  );

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
