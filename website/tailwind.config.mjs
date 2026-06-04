// ABOUTME: Tailwind configuration with Pierre design system colors
// ABOUTME: Standard Tailwind config with Pierre brand palette

/** @type {import('tailwindcss').Config} */
export default {
  content: ['./src/**/*.{astro,html,js,jsx,md,mdx,svelte,ts,tsx,vue}'],
  theme: {
    extend: {
      colors: {
        pierre: {
          violet: '#00241a',
          cyan: '#0d3b2e',
          activity: '#0f7d68',
          nutrition: '#b08326',
          recovery: '#3e7283',
          dark: '#11130f',
          slate: '#1d201d',
          'violet-light': '#a3d0be',
          'violet-dark': '#234e40',
          'cyan-light': '#beedd9',
          'activity-dark': '#0b5d4d',
          'nutrition-dark': '#8a6420',
          'recovery-dark': '#2f5664',
        },
      },
      fontFamily: {
        sans: ['Inter', 'system-ui', '-apple-system', 'Segoe UI', 'Roboto', 'sans-serif'],
        mono: ['JetBrains Mono', 'Monaco', 'Menlo', 'monospace'],
      },
    },
  },
  plugins: [],
};
