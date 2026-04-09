// ABOUTME: i18n utility functions for language detection and translation lookup
// ABOUTME: Follows the Astro i18n recipe — URL-based locale detection with EN default

import { ui, defaultLang } from './ui';

export type Lang = keyof typeof ui;

export function getLangFromUrl(url: URL): Lang {
  const [, first] = url.pathname.split('/');
  if (first in ui) return first as Lang;
  return defaultLang;
}

export function useTranslations(lang: Lang) {
  return function t(key: keyof (typeof ui)[typeof defaultLang]): string {
    return (ui[lang] as Record<string, string>)[key] ?? ui[defaultLang][key];
  };
}

/** Returns the alternate-language path for a given path and current lang. */
export function getAlternatePath(pathname: string, currentLang: Lang): string {
  if (currentLang === defaultLang) {
    return `/fr${pathname === '/' ? '' : pathname}` || '/fr';
  }
  // Strip the leading /fr prefix
  const stripped = pathname.replace(/^\/fr/, '') || '/';
  return stripped;
}

/** Resolves the correct docs collection name for a given language. */
export function getDocsCollection(lang: Lang): 'docs' | 'docs-fr' {
  return lang === 'fr' ? 'docs-fr' : 'docs';
}

/** Resolves the base docs path for a given language. */
export function getDocsBasePath(lang: Lang): string {
  return lang === 'fr' ? '/fr/docs' : '/docs';
}
