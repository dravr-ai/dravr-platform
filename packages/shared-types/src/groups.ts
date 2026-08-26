// ABOUTME: Shared TypeScript types for group coaching features
// ABOUTME: Groups, members, invites, roles, analytics, and request/response types

// ========== ENUMS ==========

/** Role within a coaching group */
export type GroupRole = 'owner' | 'admin' | 'member';

/** What redeeming a group invite grants: athlete membership or coach attachment */
export type GroupInviteKind = 'member' | 'coach';

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
  | 'deep_fatigue'
  | 'inactive'
  | 'volume_drop';

/** Severity level for health flags */
export type HealthFlagSeverity = 'info' | 'warning' | 'critical';

/** When the group's AI coach replies in the bound channel chat */
export type GroupRespondMode = 'all' | 'mentions';

// ========== CORE TYPES ==========

/** A coaching group binding a coach persona to multiple athletes */
export interface CoachingGroup {
  id: string;
  tenant_id: string;
  name: string;
  description: string | null;
  coach_id: string;
  owner_id: string;
  /** Human professional coach attached to oversee this group, if any */
  coach_user_id: string | null;
  peer_data_sharing: boolean;
  /** 'all' answers every member message; 'mentions' only explicitly-addressed ones */
  respond_mode: GroupRespondMode;
  max_members: number;
  is_active: boolean;
  created_at: string;
  updated_at: string;
}

/**
 * A member within a coaching group, shaped as the members endpoint serialises it.
 * The server lists active members only — it filters on `left_at IS NULL` — and
 * keeps `tenant_id` and `left_at` off the wire, so neither belongs here.
 */
export interface GroupMember {
  id: string;
  group_id: string;
  user_id: string;
  role: GroupRole;
  peer_sharing_consent: boolean;
  consent_given_at: string;
  joined_at: string;
  display_name: string | null;
}

/** An invite code for joining a group */
export interface GroupInvite {
  id: string;
  group_id: string;
  tenant_id: string;
  code: string;
  /** Whether redeeming this invite grants membership or coach attachment */
  kind: GroupInviteKind;
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
  coach_id?: string;
  max_members?: number;
  peer_data_sharing?: boolean;
  respond_mode?: GroupRespondMode;
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
  /** Athlete membership (default) or coach attachment */
  kind?: GroupInviteKind;
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

/** Response for listing the groups a user is the human coach of */
export interface CoachedGroupsResponse {
  groups: CoachingGroup[];
  total: number;
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

/** Response for a group's weekly report */
export interface GroupWeeklyReportResponse {
  report: GroupWeeklyReport;
}

/** Response for a group's member health flags */
export interface GroupHealthFlagsResponse {
  flags: GroupHealthFlag[];
  total: number;
}

/** Group creation permission check result */
export interface GroupPermissionsResponse {
  can_create: boolean;
  policy: string;
  /**
   * Whether the tenant's plan tier enables the weekly digest. Same flag the
   * digest scheduler sweeps on; the group detail surfaces render the weekly
   * report and health-flag panels off it rather than deriving a tier locally.
   */
  weekly_digest: boolean;
}

/**
 * One roster row of the group transcript. Membership is never hidden --
 * an unconsented member appears here with `peer_sharing_consent: false`
 * while their entries are withheld from the transcript itself.
 */
export interface TranscriptMember {
  user_id: string;
  display_name: string | null;
  role: string;
  peer_sharing_consent: boolean;
}

/** One utterance of the shared room, oldest first in the listing. */
export interface GroupTranscriptEntry {
  id: string;
  author_user_id: string;
  author_display_name: string | null;
  /** `member` or `coach` */
  speaker: string;
  content: string;
  created_at: string;
}

/** The shared room view every surface reads: roster plus visible entries. */
export interface GroupTranscriptResponse {
  group_id: string;
  members: TranscriptMember[];
  entries: GroupTranscriptEntry[];
}
