// ABOUTME: Handler for /group reads — the member's own record of their human coach reading their data, per group, newest first
// ABOUTME: In a group's chat that group; from a direct message every group the member belongs to (carnet#1445)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use dravr_canot::commands::CommandResponse;
use pierre_contremaitre::messaging_strings::{
    KEY_GROUP_READS_FORMER_COACH, KEY_GROUP_READS_HEADER, KEY_GROUP_READS_LINE,
    KEY_GROUP_READS_NONE, KEY_GROUP_READS_SOURCE_COACH_THREAD, KEY_GROUP_READS_SOURCE_GROUP_HEALTH,
    KEY_GROUP_READS_SOURCE_GROUP_REPORT, KEY_GROUP_READS_SOURCE_GROUP_ROOM,
    KEY_GROUP_READS_SOURCE_GROUP_STATS, KEY_GROUP_READS_SOURCE_PLAN_COMMAND,
    KEY_GROUP_READS_SOURCE_PLAN_VIEW, KEY_GROUP_READS_SOURCE_ROSTER,
    KEY_GROUP_READS_SOURCE_WEEKLY_DIGEST, KEY_GROUP_READS_TRAINING, KEY_GROUP_READS_WELLNESS,
};
use pierre_core::errors::AppError;
use pierre_database::repositories::{CoachReadClass, CoachReadSurface};
use pierre_services::athlete_display_name::fetch_user_display_name;
use pierre_services::coach_read_retention::{own_coach_reads, COACH_READ_RETENTION_MONTHS};

use crate::group::{resolve_consent_targets, CallerGroupStanding, ConsentTargets};
use crate::{CommandHandler, PlatformCommandContext};

/// Reads listed per group: the newest; Group info lists more.
const READS_PER_GROUP: u32 = 10;

/// Handler for `/group reads` — which of the member's data their group's
/// human coach read through their agent, through which tool, and when.
///
/// The member's own rows only, read under each tenant they belong to
/// ([`own_coach_reads`], as Group info lists them). In a group's chat it
/// lists that group; from a direct message, every group the member belongs
/// to.
#[derive(Debug)]
pub struct GroupReadsHandler;

#[async_trait]
impl CommandHandler for GroupReadsHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let repos = ctx.ctx.repos();

        let groups = match resolve_consent_targets(ctx).await? {
            ConsentTargets::Bound(group) => vec![group],
            ConsentTargets::Direct(groups) => groups,
        };
        let months = COACH_READ_RETENTION_MONTHS.to_string();

        let mut sections = Vec::with_capacity(groups.len());
        for group in groups {
            let reads = own_coach_reads(repos, ctx.user_id, group.id, READS_PER_GROUP).await?;
            if reads.is_empty() {
                sections.push(reg.render(KEY_GROUP_READS_NONE, locale, &[&group.name, &months]));
                continue;
            }
            let mut lines = vec![reg.render(KEY_GROUP_READS_HEADER, locale, &[&group.name])];
            for read in &reads {
                let coach = match read.coach {
                    Some(coach) => fetch_user_display_name(repos, coach).await,
                    None => reg.render(KEY_GROUP_READS_FORMER_COACH, locale, &[]),
                };
                let class = reg.render(
                    match read.class {
                        CoachReadClass::Training => KEY_GROUP_READS_TRAINING,
                        CoachReadClass::Wellness => KEY_GROUP_READS_WELLNESS,
                    },
                    locale,
                    &[],
                );
                // A tool is named as it is; a read made outside a tool call
                // reads in words.
                let source = CoachReadSurface::of_label(&read.tool).map_or_else(
                    || read.tool.clone(),
                    |surface| reg.render(surface_key(surface), locale, &[]),
                );
                let when = read.read_at.format("%Y-%m-%d %H:%M UTC").to_string();
                lines.push(reg.render(
                    KEY_GROUP_READS_LINE,
                    locale,
                    &[&when, &coach, &class, &source],
                ));
            }
            sections.push(lines.join("\n"));
        }
        Ok(CommandResponse::text(sections.join("\n\n")))
    }

    /// Lists the conversation's group for a member of it, or the caller's own
    /// groups from a direct message.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some() || (standing.is_direct_message && standing.highest.is_some())
    }
}

/// The words a read made outside a tool call reads as, in place of a tool.
const fn surface_key(surface: CoachReadSurface) -> &'static str {
    match surface {
        CoachReadSurface::PlanView => KEY_GROUP_READS_SOURCE_PLAN_VIEW,
        CoachReadSurface::PlanCommand => KEY_GROUP_READS_SOURCE_PLAN_COMMAND,
        CoachReadSurface::Roster => KEY_GROUP_READS_SOURCE_ROSTER,
        CoachReadSurface::CoachThread => KEY_GROUP_READS_SOURCE_COACH_THREAD,
        CoachReadSurface::GroupRoom => KEY_GROUP_READS_SOURCE_GROUP_ROOM,
        CoachReadSurface::GroupStats => KEY_GROUP_READS_SOURCE_GROUP_STATS,
        CoachReadSurface::GroupReport => KEY_GROUP_READS_SOURCE_GROUP_REPORT,
        CoachReadSurface::GroupHealth => KEY_GROUP_READS_SOURCE_GROUP_HEALTH,
        CoachReadSurface::WeeklyDigest => KEY_GROUP_READS_SOURCE_WEEKLY_DIGEST,
    }
}
