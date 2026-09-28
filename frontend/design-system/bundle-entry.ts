// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Design System artifact's bundle entry — the web primitives it previews, assigned to window.Boreal
// ABOUTME: scripts/design-system/generate.ts builds it as one IIFE over the page's React globals; each import is one card

import { Badge } from '../src/components/ui/Badge';
import { Button } from '../src/components/ui/Button';
import { Card, CardHeader } from '../src/components/ui/Card';
import { Checkbox, Radio } from '../src/components/ui/Checkbox';
import { EmptyState } from '../src/components/ui/EmptyState';
import { IconButton } from '../src/components/ui/IconButton';
import { Input } from '../src/components/ui/Input';
import { SearchField } from '../src/components/ui/SearchField';
import { Section } from '../src/components/ui/Section';
import { StatusIndicator } from '../src/components/ui/StatusIndicator';
import { TabHeader } from '../src/components/ui/TabHeader';
import { TabPanel, Tabs } from '../src/components/ui/Tabs';
import MessageBubble from '../src/components/chat/MessageBubble';
import { DravrLogo } from '../src/components/DravrLogo';

(window as unknown as Record<string, unknown>).Boreal = {
  Badge,
  Button,
  Card,
  CardHeader,
  Checkbox,
  DravrLogo,
  EmptyState,
  IconButton,
  Input,
  MessageBubble,
  Radio,
  SearchField,
  Section,
  StatusIndicator,
  TabHeader,
  TabPanel,
  Tabs,
};
