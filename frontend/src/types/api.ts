// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Re-exports shared types for the web frontend
// ABOUTME: All types are now centralized in @pierre/shared-types

// ========== ADMIN TYPES ==========
// Admin tokens, A2A protocol, dashboard analytics

export type {
  AdminPermission,
  AdminToken,
  AdminTokensResponse,
  CreateAdminTokenRequest,
  CreateAdminTokenResponse,
  ToolUsageBreakdown,
  A2AClient,
  A2AClientRegistrationRequest,
  A2AClientCredentials,
  A2ARateLimitStatus,
  A2AUsageStats,
} from '@pierre/shared-types';

// ========== AUTH TYPES ==========
// Users, login, OAuth

export type {
  UserRole,
  UserStatus,
  UserTier,
  User,
  AdminUser,
  LoginResponse,
  RegisterResponse,
  FirebaseLoginResponse,
  ProviderStatus,
  OAuthApp,
  OAuthAppCredentials,
  OAuthProvider,
  McpToken,
  UserManagementResponse,
  ApproveUserRequest,
  SuspendUserRequest,
} from '@pierre/shared-types';

// ========== COACH TYPES ==========
// Agent personas, store, versions

export type {
  AgentCategory,
  AgentVisibility,
  PublishStatus,
  Agent,
  UpdateAgentRequest,
  AgentMetadata,
  ListAgentsResponse,
  StoreAgent,
  StoreAgentDetail,
  StoreMetadata,
  BrowseAgentsResponse,
  SearchAgentsResponse,
  InstallAgentResponse,
  UninstallAgentResponse,
  InstallationsResponse,
  AgentAssignment,
  AssignAgentResponse,
  UnassignAgentResponse,
  ListAssignmentsResponse,
} from '@pierre/shared-types';

// ========== API TYPES ==========
// Chat, prompts, common patterns

export type {
  Conversation,
  Message,
  ActivityPillar,
  ApiMetadata,
  PaginatedResponse,
  ListResponse,
} from '@pierre/shared-types';
