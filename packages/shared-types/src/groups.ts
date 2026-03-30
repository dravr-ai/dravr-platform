// ABOUTME: Shared TypeScript types for group coaching features
// ABOUTME: Groups, members, invites, roles, analytics, and request/response types

// ========== ENUMS ==========

/** Role within a coaching group */
export type GroupRole = 'owner' | 'admin' | 'member';

/** Overtraining risk level for a member */
export type OvertrainingRiskLevel = 'low' | 'moderate' | 'high';

/** Group-level metric trend direction */
export type GroupTrend = 'improving' | 'stable' | 'declining';

/** Detail level for member summary generation */
export type SummaryDetailLevel = 'roster' | 'weekly' | 'detailed';

/** Alert flag type for group members */
export type MemberFlag =
  | 'overreaching'
  | 'fresh_form'
  | 'personal_record'
  | 'injury_risk'
  | 'inactive';

/** Severity level for health flags */
export type HealthFlagSeverity = 'info' | 'warning' | 'critical';

// ========== CORE TYPES ==========

/** A coaching group binding a coach persona to multiple athletes */
export interface CoachingGroup {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  coach_id: string;
  owner_id: string;
  peer_data_sharing: boolean;
  max_members: number;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

/** A member within a coaching group */
export interface GroupMember {
  id: string;
  group_id: string;
  user_id: string;
  tenant_id: string;
  role: GroupRole;
  peer_sharing_consent: boolean;
  consent_given_at: string;
  joined_at: string;
  left_at: string | null;
  display_name: string | null;
}

/** An invite code for joining a group */
export interface GroupInvite {
  id: string;
  group_id: string;
  tenant_id: string;
  code: string;
  created_by: string;
  expires_at: string | null;
  max_uses: number | null;
  use_count: number;
  is_active: boolean;
  created_at: string;
}

// ========== REQUEST TYPES ==========

/** Request to create a new coaching group */
export interface CreateGroupRequest {
  name: string;
  description?: string;
  coach_id: string;
  max_members?: number;
}

/** Request to update a coaching group */
export interface UpdateGroupRequest {
  name?: string;
  description?: string;
  max_members?: number;
  peer_data_sharing?: boolean;
  is_active?: boolean;
}

/** Request to join a group via invite code */
export interface JoinGroupRequest {
  invite_code: string;
}

/** Request to update a member's role */
export interface UpdateMemberRoleRequest {
  role: GroupRole;
}

/** Request to update peer sharing consent */
export interface UpdatePeerConsentRequest {
  consent: boolean;
}

/** Request to create an invite */
export interface CreateInviteRequest {
  expires_in_days?: number;
  max_uses?: number;
}

// ========== RESPONSE TYPES ==========

/** Lightweight group summary for list views */
export interface GroupSummary {
  id: string;
  name: string;
  description: string | null;
  coach_id: string;
  member_count: number;
  is_active: boolean;
  peer_data_sharing: boolean;
  my_role: GroupRole;
  created_at: string;
}

/** Aggregate statistics for a coaching group */
export interface GroupAggregateStats {
  total_members: number;
  active_members: number;
  avg_weekly_volume_km: number;
  avg_ctl: number | null;
  flagged_members: number;
  weekly_trend: GroupTrend;
}

/** Comparison of one member against group norms */
export interface MemberGroupComparison {
  user_id: string;
  display_name: string;
  volume_percentile: number;
  ctl_percentile: number | null;
  comparison_text: string;
}

/** Health flag for a group member needing attention */
export interface GroupHealthFlag {
  user_id: string;
  display_name: string;
  flag_type: MemberFlag;
  severity: HealthFlagSeverity;
  detail: string;
}

/** Weekly report for a coaching group */
export interface GroupWeeklyReport {
  summary: string;
  highlights: string[];
  concerns: string[];
  recommendations: string[];
  stats: GroupAggregateStats;
}

// ========== LIST RESPONSES ==========

/** Response for listing groups */
export interface ListGroupsResponse {
  groups: GroupSummary[];
}

/** Response for listing group members */
export interface GroupMembersResponse {
  members: GroupMember[];
}

/** Response for listing group invites */
export interface GroupInvitesResponse {
  invites: GroupInvite[];
}

/** Response for group stats */
export interface GroupStatsResponse {
  stats: GroupAggregateStats;
}

/** Group creation permission check result */
export interface GroupPermissionsResponse {
  can_create: boolean;
  policy: string;
}
