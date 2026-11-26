// ABOUTME: The one predicate that decides whether a caller is offered a tool and may call it: scope, then transport, then role
// ABOUTME: Read by the /mcp listing, the A2A card, the chat catalogue and the dispatch chokepoint, so list and call cannot disagree

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Who is offered which tool (carnet#1511)
//!
//! Three tool lists, and the role of the person who signed in decides
//! (JF, 2026-11-12):
//!
//! - an athlete is offered the athlete tools;
//! - a human coach who leads an active group is also offered the coach tools
//!   ([`Audience::Coach`]: the roster overview, a member's activities, a
//!   member's plan status), which show only the members who allowed their
//!   coach's assistant ([`crate::coach_assistant`]);
//! - an admin is offered every tool, the coach tools with or without a group,
//!   and, on a credential whose grant holds `admin`, the admin tools and the
//!   tools kept on Dravr's own surfaces.
//!
//! The grant still narrows every list: a read-only connection stays
//! read-only, even for an admin.
//!
//! A tool is withheld from a caller on one of three axes, read in this order,
//! each from one thing the tool declares about itself:
//!
//! 1. **Scope**: how much the credential was delegated
//!    ([`missing_scope`](crate::scopes::missing_scope) over the tool's
//!    capabilities). First, so a caller that is not authorized learns nothing
//!    the later answers would tell them.
//! 2. **Transport**: where the tool may run ([`ToolReach`]). Externally an
//!    [`ToolReach::AppOnly`] tool is withheld from everyone and a
//!    [`ToolReach::FirstPartyOnly`] tool from everyone but an admin whose
//!    grant holds `admin`.
//! 3. **Role**: who the tool serves ([`Audience`]). An admin tool needs an
//!    admin on every transport; a coach tool needs a coach or an admin over
//!    an external transport only, because in Dravr's own app the tool's own
//!    gates decide (a peer reads a group member there too).
//!
//! In an admin's support session ([`crate::support_session`]) the caller is
//! the user the session acts as, and the tools that carry the user's whole
//! record away ([`RuntimeTool::exports_record`]) are withheld on top: support
//! may see what it needs to help, never take the whole record with it.
//!
//! The `athlete` argument, with which a coach names one coached athlete on a
//! read, is offered externally to a coach alone, and never on a tool that
//! writes: carnet#1533 refuses every external write that names an athlete
//! (`offers_athlete_argument`, applied by [`retain_offered`]).
//!
//! Under a detailed consent grant ([`GrantCeiling`], plan slice 3 item 6)
//! the ceiling is read right after the scope, from what the tool declares it
//! reads ([`ToolData`]): every write scope is withheld (a preset is a read
//! set); a tool reading a tier above the grant's `max_tier` is withheld
//! ([`Withheld::Tier`]); a tool is offered only when it has a declaration
//! whose streams are non-empty and all held by the grant (an incidental
//! route aside), reads no person's data, or reads only connection metadata
//! ([`Withheld::Stream`]); and a tool aggregating the whole history is
//! withheld under the grant's data window ([`Withheld::Window`]).
//!
//! Listing keeps a tool when [`withheld`] answers `None`; the dispatch
//! chokepoint refuses a call with the same answer. The role is read live on
//! each listing and each call and kept nowhere: `/mcp` holds no session, and
//! the listing carries `ttlMs: 0` with `cacheScope: private`, so a role
//! change shows at the client's next list and on the very next call.

use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::RepositoryRegistry;
use pierre_mcp_schema::ToolSchema;
use pierre_services::coach_roster::{self, RosterAccess};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use crate::protocol::types::UniversalResponse;
use crate::read_subject::{DataClass, ATHLETE_ARGUMENT};
use crate::registry::ToolRegistry;
use crate::scopes::missing_scope;
use crate::security::{Audience, RuntimeTool, ToolReach};
use crate::tool_data::ToolData;
use dravr_barrage::{Stream, Tier};
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;

/// `error_code` of the in-band refusal a coach tool returns to an external
/// caller who is neither a human coach nor an admin.
pub const COACH_ROLE_REQUIRED: &str = "coach_role_required";

/// Who the caller is: two independent flags, read live.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CallerRole {
    /// The user holds the admin role.
    pub admin: bool,
    /// The user is a human coach by the one coach rule
    /// ([`coach_roster::roster_access`]): coach access and at least one
    /// active group they coach.
    pub coach: bool,
}

impl CallerRole {
    /// The role of `user_id`, read from the database now.
    ///
    /// A failed lookup answers the flag `false`: the listing then omits what
    /// the role would add, which the next listing restores, rather than
    /// offering what the call would refuse.
    pub async fn resolve(repos: &RepositoryRegistry, user_id: Uuid) -> Self {
        let admin = match repos.users.get_global(user_id).await {
            Ok(user) => user.is_some_and(|user| user.role.is_admin()),
            Err(error) => {
                warn!(%user_id, %error, "admin lookup for a tool listing failed; reading as no admin");
                false
            }
        };
        Self {
            admin,
            coach: is_human_coach(repos, user_id).await,
        }
    }
}

/// Whether `user_id` is a human coach by the one coach rule: coach access
/// (`manages_roster`, or admin) and at least one active group they coach. A
/// failed lookup answers `false`.
pub async fn is_human_coach(repos: &RepositoryRegistry, user_id: Uuid) -> bool {
    match coach_roster::roster_access(repos, user_id).await {
        Ok(RosterAccess::Granted(_)) => true,
        Ok(RosterAccess::NoCoachAccess(_) | RosterAccess::NotACoach) => false,
        Err(error) => {
            warn!(%user_id, %error, "coach lookup for a tool listing failed; reading as no coach");
            false
        }
    }
}

/// The caller one listing or one call is decided for.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'grant> {
    /// Who they are.
    pub role: CallerRole,
    /// What their credential was granted.
    pub grant: &'grant [OAuthScope],
    /// The transport the call is served over; `None` is external, as
    /// everywhere a transport is read.
    pub transport: Option<Transport>,
    /// The call is made in an admin's support session, acting as the user
    /// (carnet#1511, carnet#1634).
    pub support_session: bool,
    /// The ceiling of the detailed consent grant the credential was minted
    /// under; `None` under a plain grant and for every other credential.
    pub ceiling: Option<&'grant GrantCeiling>,
}

impl Caller<'_> {
    /// Whether the call is served over an external transport.
    fn is_external(&self) -> bool {
        !self.transport.is_some_and(Transport::is_first_party)
    }
}

/// Why a tool is withheld from a caller: the first axis that withholds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Withheld {
    /// The grant does not cover the scope the tool needs.
    Scope(OAuthScope),
    /// The tool does not run over this transport for this caller.
    Transport,
    /// The caller lacks the role the tool's audience needs.
    Role(Audience),
    /// The tool carries the user's whole record away, and the caller is an
    /// admin's support session.
    SupportSession,
    /// The tool reads data of this tier, above the consent grant's ceiling.
    Tier(Tier),
    /// The tool reads a stream the consent grant does not hold, or declares
    /// none it holds.
    Stream,
    /// The tool aggregates the whole history, past the consent grant's data
    /// window.
    Window,
}

/// The first reason `tool`, registered in `registry` as `name`, is withheld
/// from `caller`, or `None` when it is offered and may be called. What the
/// tool reads is its declaration in `registry` ([`ToolData`]).
#[must_use]
pub fn withheld(
    registry: &ToolRegistry,
    name: &str,
    tool: &dyn RuntimeTool,
    caller: &Caller<'_>,
) -> Option<Withheld> {
    decide(declared(tool, registry.tool_data(name).copied()), caller)
}

/// [`withheld`] for one call with `args`.
///
/// What it reads is the declaration narrowed to the streams its arguments
/// pick, so a `labs` read of `get_health_records` is asked about labs alone
/// (carnet#1544 slice 4b).
#[must_use]
pub(crate) fn withheld_call(
    registry: &ToolRegistry,
    name: &str,
    tool: &dyn RuntimeTool,
    caller: &Caller<'_>,
    args: &Value,
) -> Option<Withheld> {
    decide(declared(tool, registry.call_data(name, args)), caller)
}

/// The streams a step-up would open for a call to `name` with `args`.
///
/// The call is one the consent grant `grant` was minted under with `ceiling`
/// refuses on its tier or its streams (owl §7.6): `None` when it is not so refused,
/// or a step-up cannot cure it (a T3 or raw stream, one a one-off question
/// never picks).
///
/// The one predicate of the step-up answer: the dispatch chokepoint gives it
/// in band, and `/mcp` as RFC 6750's `403 insufficient_scope` before
/// dispatch, so a host following MCP's step-up flow re-authorizes for it.
#[must_use]
pub fn step_up_cure(
    registry: &ToolRegistry,
    (name, args): (&str, &Value),
    grant: &[OAuthScope],
    ceiling: &GrantCeiling,
) -> Option<Vec<Stream>> {
    let tool = registry.get(name)?;
    let data = registry.call_data(name, args)?;
    let streams: Vec<Stream> = data.required_streams().collect();
    let call = ceiling.for_streams(&streams);
    match beyond_ceiling(&declared(tool.as_ref(), Some(data)), grant, &call) {
        Some(Withheld::Tier(_) | Withheld::Stream) => {}
        _ => return None,
    }
    ceiling.step_up_cures(&streams).then_some(streams)
}

/// What `tool` declares, reading `data`.
fn declared(tool: &dyn RuntimeTool, data: Option<ToolData>) -> Declared {
    Declared {
        surface: tool.reach(),
        audience: tool.audience(),
        capabilities: tool.capabilities(),
        exports_record: tool.exports_record(),
        data,
    }
}

/// What a tool declares about itself that the predicate reads.
#[derive(Debug, Clone, Copy)]
struct Declared {
    surface: ToolReach,
    audience: Audience,
    capabilities: ToolCapabilities,
    exports_record: bool,
    /// What it reads; `None` for a tool that declared nothing.
    data: Option<ToolData>,
}

/// [`withheld`] over what a tool declares.
fn decide(tool: Declared, caller: &Caller<'_>) -> Option<Withheld> {
    if let Some(missing) = missing_scope(caller.grant, tool.capabilities) {
        return Some(Withheld::Scope(missing));
    }
    if let Some(ceiling) = caller.ceiling {
        if let Some(beyond) = beyond_ceiling(&tool, caller.grant, ceiling) {
            return Some(beyond);
        }
    }
    if caller.support_session && tool.exports_record {
        return Some(Withheld::SupportSession);
    }
    if caller.is_external() {
        let lifted = match tool.surface {
            ToolReach::AnyTransport => true,
            ToolReach::AppOnly => false,
            ToolReach::FirstPartyOnly => {
                caller.role.admin && caller.grant.contains(&OAuthScope::Admin)
            }
        };
        if !lifted {
            return Some(Withheld::Transport);
        }
    }
    match tool.audience {
        Audience::Athlete => None,
        Audience::Admin => (!caller.role.admin).then_some(Withheld::Role(Audience::Admin)),
        Audience::Coach => (caller.is_external() && !caller.role.coach && !caller.role.admin)
            .then_some(Withheld::Role(Audience::Coach)),
    }
}

/// Why `tool` is beyond `ceiling`, the consent grant `grant` was minted
/// under, or `None` when the grant reaches it.
fn beyond_ceiling(
    tool: &Declared,
    grant: &[OAuthScope],
    ceiling: &GrantCeiling,
) -> Option<Withheld> {
    // A preset is a read set: every write scope is withheld.
    let read_only: Vec<OAuthScope> = grant
        .iter()
        .copied()
        .filter(|scope| {
            !matches!(
                scope,
                OAuthScope::FitnessWrite | OAuthScope::ProfileWrite | OAuthScope::Admin
            )
        })
        .collect();
    if let Some(missing) = missing_scope(&read_only, tool.capabilities) {
        return Some(Withheld::Scope(missing));
    }
    let Some(data) = tool.data else {
        return Some(Withheld::Stream);
    };
    if let Some(tier) = data
        .required_streams()
        .map(Stream::tier)
        .filter(|tier| !ceiling.reaches(*tier))
        .min()
    {
        return Some(Withheld::Tier(tier));
    }
    if data.class() != Some(DataClass::NoSubject) && !data.connection_metadata() {
        let mut streams = data.required_streams().peekable();
        if streams.peek().is_none() || !streams.all(|stream| ceiling.holds(stream)) {
            return Some(Withheld::Stream);
        }
    }
    data.whole_history().then_some(Withheld::Window)
}

/// Whether `caller` is offered the `athlete` argument on `tool`.
///
/// On Dravr's own surfaces nothing changes: the argument is the tool's own,
/// and chat refuses it itself. Externally it goes to a human coach alone, and
/// never on a tool that writes: an external write that names an athlete is
/// refused for every role (carnet#1533).
#[must_use]
fn offers_athlete_argument(tool: &dyn RuntimeTool, caller: &Caller<'_>) -> bool {
    offers_athlete(tool.capabilities(), caller)
}

/// [`offers_athlete_argument`] over a tool's capabilities.
fn offers_athlete(capabilities: ToolCapabilities, caller: &Caller<'_>) -> bool {
    !caller.is_external()
        || (caller.role.coach && !capabilities.contains(ToolCapabilities::WRITES_DATA))
}

/// Narrow a built listing to what `caller` is offered.
///
/// Every tool [`withheld`] withholds goes, and so does the `athlete`
/// property of every tool left that does not offer it: externally a human
/// coach alone is offered it, and never on a tool that writes.
pub fn retain_offered(registry: &ToolRegistry, schemas: &mut Vec<ToolSchema>, caller: &Caller<'_>) {
    schemas.retain(|schema| {
        registry
            .get(&schema.name)
            .is_none_or(|tool| withheld(registry, &schema.name, tool.as_ref(), caller).is_none())
    });
    for schema in schemas.iter_mut() {
        let offered = registry
            .get(&schema.name)
            .is_none_or(|tool| offers_athlete_argument(tool.as_ref(), caller));
        if offered {
            continue;
        }
        if let Some(properties) = &mut schema.input_schema.properties {
            properties.remove(ATHLETE_ARGUMENT);
        }
        if let Some(required) = &mut schema.input_schema.required {
            required.retain(|name| name != ATHLETE_ARGUMENT);
        }
    }
}

/// The in-band refusal of a coach tool an external caller is not a coach
/// for. Never a scope challenge: no scope would fix it, and a client would
/// ask for one in a loop.
#[must_use]
pub fn coach_role_refusal(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": COACH_ROLE_REQUIRED,
            "reason": COACH_ROLE_REQUIRED,
        })),
        error: Some(format!(
            "'{tool_name}' is for a human coach who leads an active group in Dravr. The account \
             connected here leads none."
        )),
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: ToolCapabilities = ToolCapabilities::READS_DATA;
    const WRITE: ToolCapabilities = ToolCapabilities::WRITES_DATA;

    const fn tool(
        surface: ToolReach,
        audience: Audience,
        capabilities: ToolCapabilities,
    ) -> Declared {
        Declared {
            surface,
            audience,
            capabilities,
            exports_record: false,
            data: None,
        }
    }

    const ATHLETE: CallerRole = CallerRole {
        admin: false,
        coach: false,
    };
    const COACH: CallerRole = CallerRole {
        admin: false,
        coach: true,
    };
    const ADMIN: CallerRole = CallerRole {
        admin: true,
        coach: false,
    };

    const READ_ONLY: [OAuthScope; 2] = [OAuthScope::FitnessRead, OAuthScope::ProfileRead];

    const fn caller(role: CallerRole, grant: &[OAuthScope], transport: Transport) -> Caller<'_> {
        Caller {
            role,
            grant,
            transport: Some(transport),
            support_session: false,
            ceiling: None,
        }
    }

    #[test]
    fn the_coach_tools_go_externally_to_a_coach_and_an_admin_only() {
        let roster = tool(ToolReach::AnyTransport, Audience::Coach, READ);
        assert_eq!(
            decide(roster, &caller(ATHLETE, &READ_ONLY, Transport::McpHttp)),
            Some(Withheld::Role(Audience::Coach)),
            "bug: a coach tool reached an athlete over MCP"
        );
        assert_eq!(
            decide(roster, &caller(COACH, &READ_ONLY, Transport::McpHttp)),
            None,
            "bug: a coach is offered the coach tools over MCP"
        );
        assert_eq!(
            decide(roster, &caller(ADMIN, &READ_ONLY, Transport::A2a)),
            None,
            "bug: an admin is offered the coach tools over A2A"
        );
        assert_eq!(
            decide(roster, &caller(ATHLETE, &READ_ONLY, Transport::WebApp)),
            None,
            "in Dravr's own app the tool's own gates decide"
        );
    }

    #[test]
    fn a_first_party_only_tool_goes_externally_to_an_admin_holding_admin_alone() {
        let agents = tool(ToolReach::FirstPartyOnly, Audience::Athlete, READ);
        let with_admin = OAuthScope::self_grant();
        assert_eq!(
            decide(agents, &caller(ADMIN, &READ_ONLY, Transport::McpHttp)),
            Some(Withheld::Transport),
            "a read-only connection holds no admin, so an admin is not lifted (Q25)"
        );
        assert_eq!(
            decide(agents, &caller(ADMIN, &with_admin, Transport::McpHttp)),
            None,
            "bug: an admin holding admin is offered a first-party tool externally (Q25)"
        );
        assert_eq!(
            decide(agents, &caller(COACH, &with_admin, Transport::ApiKey)),
            Some(Withheld::Transport),
            "bug: a coach is never lifted past a first-party tool's transport"
        );
        assert_eq!(
            decide(agents, &caller(ATHLETE, &READ_ONLY, Transport::MobileApp)),
            None,
            "bug: Dravr's own app never withholds a tool by transport"
        );
    }

    #[test]
    fn an_app_only_tool_is_withheld_externally_from_everyone() {
        let publish = tool(ToolReach::AppOnly, Audience::Athlete, WRITE);
        let every = OAuthScope::self_grant();
        for role in [ATHLETE, COACH, ADMIN] {
            assert_eq!(
                decide(publish, &caller(role, &every, Transport::ApiKey)),
                Some(Withheld::Transport),
                "bug: an app-only tool reached an external transport"
            );
            assert_eq!(
                decide(publish, &caller(role, &every, Transport::WebApp)),
                None,
                "bug: Dravr's own app offers an app-only tool"
            );
        }
    }

    #[test]
    fn scope_is_read_first_and_an_admin_tool_needs_an_admin_everywhere() {
        let admin_tool = tool(
            ToolReach::AnyTransport,
            Audience::Admin,
            ToolCapabilities::ADMIN_ONLY,
        );
        assert_eq!(
            decide(admin_tool, &caller(ADMIN, &READ_ONLY, Transport::McpHttp)),
            Some(Withheld::Scope(OAuthScope::Admin)),
            "bug: scope is read first: a grant without admin is refused on scope"
        );
        let every = OAuthScope::self_grant();
        assert_eq!(
            decide(admin_tool, &caller(ADMIN, &every, Transport::McpHttp)),
            None,
            "bug: an admin holding admin is offered an admin tool"
        );
        assert_eq!(
            decide(admin_tool, &caller(COACH, &every, Transport::WebApp)),
            Some(Withheld::Role(Audience::Admin)),
            "bug: an admin tool reached a coach in the app"
        );
    }

    #[test]
    fn the_athlete_argument_goes_externally_to_a_coach_on_reads_alone() {
        let every = OAuthScope::self_grant();
        assert!(
            offers_athlete(READ, &caller(COACH, &every, Transport::McpHttp)),
            "bug: a coach's external read offers athlete="
        );
        assert!(
            !offers_athlete(WRITE, &caller(COACH, &every, Transport::McpHttp)),
            "bug: a write never offers athlete= externally"
        );
        assert!(
            !offers_athlete(READ, &caller(ADMIN, &every, Transport::McpHttp)),
            "bug: an admin who coaches no group is not offered athlete="
        );
        assert!(
            !offers_athlete(READ, &caller(ATHLETE, &every, Transport::A2a)),
            "bug: an athlete is not offered athlete= externally"
        );
        assert!(
            offers_athlete(WRITE, &caller(ATHLETE, &every, Transport::WebApp)),
            "bug: in the app the tool's own gates decide athlete="
        );
    }

    #[test]
    fn an_undeclared_transport_is_external() {
        let roster = tool(ToolReach::AnyTransport, Audience::Coach, READ);
        let undeclared = Caller {
            role: ATHLETE,
            grant: &READ_ONLY,
            transport: None,
            support_session: false,
            ceiling: None,
        };
        assert_eq!(
            decide(roster, &undeclared),
            Some(Withheld::Role(Audience::Coach)),
            "bug: an undeclared transport reads as external"
        );
    }

    /// Over every declaration, role, grant and transport: no admin tool
    /// reaches a non-admin, and Dravr's own surfaces never withhold a tool by
    /// transport.
    #[test]
    fn no_admin_tool_reaches_a_non_admin_and_the_app_withholds_nothing_by_transport() {
        let every = OAuthScope::self_grant();
        let grants: [&[OAuthScope]; 3] = [
            &READ_ONLY,
            &[OAuthScope::FitnessRead, OAuthScope::FitnessWrite],
            &every,
        ];
        let transports = [
            Transport::McpHttp,
            Transport::A2a,
            Transport::ApiKey,
            Transport::WebApp,
        ];
        let surfaces = [
            ToolReach::AnyTransport,
            ToolReach::FirstPartyOnly,
            ToolReach::AppOnly,
        ];
        let audiences = [Audience::Athlete, Audience::Coach, Audience::Admin];
        for surface in surfaces {
            for audience in audiences {
                let caps = if audience == Audience::Admin {
                    ToolCapabilities::ADMIN_ONLY
                } else {
                    READ
                };
                let declared = tool(surface, audience, caps);
                for role in [ATHLETE, COACH, ADMIN] {
                    for grant in grants {
                        for transport in transports {
                            let answer = decide(declared, &caller(role, grant, transport));
                            if transport.is_first_party() {
                                assert_ne!(
                                    answer,
                                    Some(Withheld::Transport),
                                    "bug: Dravr's own surfaces withheld a tool by transport"
                                );
                            }
                            if audience == Audience::Admin && !role.admin {
                                assert!(answer.is_some(), "an admin tool reached a non-admin");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_support_session_is_withheld_the_tools_that_carry_the_record_away() {
        let export = Declared {
            exports_record: true,
            ..tool(ToolReach::AnyTransport, Audience::Athlete, READ)
        };
        let every = OAuthScope::delegable_grant();
        let mut in_session = caller(ATHLETE, &every, Transport::McpHttp);
        assert_eq!(decide(export, &in_session), None, "the user's own call");
        in_session.support_session = true;
        assert_eq!(
            decide(export, &in_session),
            Some(Withheld::SupportSession),
            "bug: a support session is withheld the export"
        );
        assert_eq!(
            decide(
                tool(ToolReach::AnyTransport, Audience::Athlete, READ),
                &in_session
            ),
            None,
            "everything else the user sees is served"
        );
    }

    mod under_a_preset {
        use super::*;
        use dravr_barrage::Preset;
        use dravr_barrage::Stream::{
            Fitness, Labs, Profile, RestrictedNotes, Sleep, Workout, WorkoutRaw, WorkoutRoute,
        };
        use dravr_tronc::dates::{Timestamp, Zone};
        use pierre_auth::oauth2_server::authorization_details::preset_defaults;

        fn ceiling(preset: Preset) -> GrantCeiling {
            let details = preset_defaults(preset, "https://mcp.example.test/mcp")
                .freeze(Timestamp::now(), &Zone::utc())
                .unwrap()
                .0;
            GrantCeiling::of_details("g", &details, &Zone::utc()).unwrap()
        }

        fn reads(capabilities: ToolCapabilities, data: ToolData) -> Declared {
            Declared {
                data: Some(data),
                ..tool(ToolReach::AnyTransport, Audience::Athlete, capabilities)
            }
        }

        fn of(streams: &'static [Stream]) -> ToolData {
            ToolData::of(DataClass::Training, streams)
        }

        fn under<'a>(grant: &'a [OAuthScope], ceiling: &'a GrantCeiling) -> Caller<'a> {
            Caller {
                ceiling: Some(ceiling),
                ..caller(ATHLETE, grant, Transport::McpHttp)
            }
        }

        const SESSIONS: &[Stream] = &[Workout];
        const SAMPLES: &[Stream] = &[Workout, WorkoutRaw];
        const PLACED: &[Stream] = &[Workout, WorkoutRoute];
        const RECORDS: &[Stream] = &[Labs, RestrictedNotes];
        const MEMORY: &[Stream] = &[RestrictedNotes];
        const NIGHTS: &[Stream] = &[Sleep];
        const PHYSIOLOGY: &[Stream] = &[Fitness, Profile];

        #[test]
        fn a_training_grant_reads_its_streams_and_nothing_else() {
            let training = ceiling(Preset::TrainingCoach);
            let every = OAuthScope::delegable_grant();
            let caller = under(&every, &training);
            assert_eq!(
                decide(reads(READ, of(SESSIONS)), &caller),
                None,
                "bug: a training grant was refused its sessions"
            );
            assert_eq!(
                decide(reads(READ, of(SAMPLES)), &caller),
                None,
                "summary_and_raw"
            );
            assert_eq!(
                decide(reads(READ, of(PHYSIOLOGY)), &caller),
                None,
                "bug: a training grant was refused the athlete's physiology"
            );
            assert_eq!(
                decide(reads(READ, of(PLACED)), &caller),
                Some(Withheld::Stream),
                "bug: a tool whose answer is the place reached a grant without routes"
            );
            let incidental = ToolData::of(DataClass::Training, PLACED).with_incidental_route();
            assert_eq!(
                decide(reads(READ, incidental), &caller),
                None,
                "an incidental route is served without its coordinates"
            );
            let undeclared = tool(ToolReach::AnyTransport, Audience::Athlete, READ);
            assert_eq!(
                decide(undeclared, &caller),
                Some(Withheld::Stream),
                "bug: a tool declaring nothing reached a preset grant"
            );
            assert_eq!(
                decide(reads(READ, of(&[])), &caller),
                Some(Withheld::Stream),
                "a tool declaring no stream reads no preset's data"
            );
            let reference = ToolData::of(DataClass::NoSubject, &[]);
            assert_eq!(
                decide(reads(READ, reference), &caller),
                None,
                "bug: a tool reading no person's data was refused a preset grant"
            );
            let connections = ToolData::of(DataClass::Training, &[]).with_connection_metadata();
            assert_eq!(
                decide(reads(READ, connections), &caller),
                None,
                "which sources are connected is offered under any preset"
            );
            let own_connections = ToolData::own(&[]).with_connection_metadata();
            assert_eq!(
                decide(reads(READ, own_connections), &caller),
                None,
                "bug: the athlete's own connections were refused a preset grant"
            );
            let history = ToolData::of(DataClass::Training, SESSIONS).with_whole_history();
            assert_eq!(
                decide(reads(READ, history), &caller),
                Some(Withheld::Window),
                "bug: a whole-history total reached a grant's window"
            );
        }

        #[test]
        fn the_tier_answers_before_the_streams_and_writes_are_withheld_on_scope() {
            let training = ceiling(Preset::TrainingCoach);
            let every = OAuthScope::delegable_grant();
            let preset = under(&every, &training);
            assert_eq!(
                decide(reads(READ, of(RECORDS)), &preset),
                Some(Withheld::Tier(Tier::T2)),
                "bug: health records were refused on their streams, not tier_exceeds_grant"
            );
            assert_eq!(
                decide(reads(READ, of(MEMORY)), &preset),
                Some(Withheld::Tier(Tier::T3)),
                "bug: a T3 read was not refused on its tier"
            );
            assert_eq!(
                decide(reads(WRITE, of(SESSIONS)), &preset),
                Some(Withheld::Scope(OAuthScope::FitnessWrite)),
                "bug: a preset grant wrote"
            );
            assert_eq!(
                decide(
                    reads(WRITE, of(SESSIONS)),
                    &caller(ATHLETE, &every, Transport::McpHttp)
                ),
                None,
                "a plain grant still writes"
            );
            let read_only = READ_ONLY;
            assert_eq!(
                decide(reads(WRITE, of(RECORDS)), &under(&read_only, &training)),
                Some(Withheld::Scope(OAuthScope::FitnessWrite)),
                "the scope answers first"
            );
        }

        #[test]
        fn a_sleep_grant_reads_no_session() {
            let sleep = ceiling(Preset::SleepCoach);
            let every = OAuthScope::delegable_grant();
            let caller = under(&every, &sleep);
            assert_eq!(
                decide(reads(READ, of(NIGHTS)), &caller),
                None,
                "bug: a sleep grant was refused its nights"
            );
            assert_eq!(
                decide(reads(READ, of(SESSIONS)), &caller),
                Some(Withheld::Stream),
                "bug: a sleep grant read sessions"
            );
            assert_eq!(
                decide(reads(READ, of(SAMPLES)), &caller),
                Some(Withheld::Stream),
                "bug: a sleep grant read workout samples"
            );
        }
    }
}
