// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Web app entry point — mounts React, Chart.js and the i18n runtime
// ABOUTME: Registers the locale writer that keeps chrome and reply language equal

import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { initI18n } from '@pierre/i18n'
import './index.css'
import App from './App.tsx'
import { persistLocale } from './i18n/localePersister'
import { i18nApi } from './services/api'

// One preference, two owners: i18next renders the chrome, `users.locale`
// decides the language the coach answers in. Registering the writer here — the
// only place the app is constructed — means every language change made
// anywhere in the app reaches the server, instead of stopping at localStorage.
// The fetcher is the other direction: the live catalogue overlays the embedded
// copy, so a string fixed upstream reaches the app without a deploy.
initI18n({ persistLocale, fetchBundle: i18nApi.bundle })

// Register Chart.js components globally
import {
  Chart as ChartJS,
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  ArcElement,
  Title,
  Tooltip,
  Legend,
  Filler,
} from 'chart.js';

ChartJS.register(
  CategoryScale,
  LinearScale,
  PointElement,
  LineElement,
  BarElement,
  ArcElement,
  Title,
  Tooltip,
  Legend,
  Filler,
);

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
