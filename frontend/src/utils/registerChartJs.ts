// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Registers the Chart.js elements and plugins the admin charts draw with, once
// ABOUTME: Imported by each chart panel, so chart.js loads with those lazy chunks and never in the entry bundle

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

// Registered from the entry, chart.js would add ~170 KB to the bundle every
// athlete downloads before first paint, for charts only the admin console draws.
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
