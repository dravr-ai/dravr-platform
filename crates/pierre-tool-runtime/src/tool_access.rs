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
//! The `athlete` argument, with which a coach names one coached athlete on a
//! read, is offered externally to a coach alone, and never on a tool that
//! writes: carnet#1533 refuses every external write that names an athlete
//! ([`offers_athlete_argument`]).
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
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::protocol::types::UniversalResponse;
use crate::read_subject::ATHLETE_ARGUMENT;
use crate::registry::ToolRegistry;
use crate::scopes::missing_scope;
use crate::security::{Audience, RuntimeTool, ToolReach};
use dravr_tronc::mcp::tool::ToolCapabilities;

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
pub struct Caller<'a> {
    /// Who they are.
    pub role: CallerRole,
    /// What their credential was granted.
    pub grant: &'a [OAuthScope],
    /// The transport the call is served over; `None` is external, as
    /// everywhere a transport is read.
    pub transport: Option<Transport>,
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
}

/// The first reason `tool` is withheld from `caller`, or `None` when it is
/// offered and may be called.
#[must_use]
pub fn withheld(tool: &dyn RuntimeTool, caller: &Caller<'_>) -> Option<Withheld> {
    decide(
        Declared {
            surface: tool.reach(),
            audience: tool.audience(),
            capabilities: tool.capabilities(),
        },
        caller,
    )
}

/// What a tool declares about itself that the predicate reads.
#[derive(Debug, Clone, Copy)]
struct Declared {
    surface: ToolReach,
    audience: Audience,
    capabilities: ToolCapabilities,
}

/// [`withheld`] over what a tool declares.
fn decide(tool: Declared, caller: &Caller<'_>) -> Option<Withheld> {
    if let Some(missing) = missing_scope(caller.grant, tool.capabilities) {
        return Some(Withheld::Scope(missing));
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

/// Whether `caller` is offered the `athlete` argument on `tool`.
///
/// On Dravr's own surfaces nothing changes: the argument is the tool's own,
/// and chat refuses it itself. Externally it goes to a human coach alone, and
/// never on a tool that writes: an external write that names an athlete is
/// refused for every role (carnet#1533).
#[must_use]
pub fn offers_athlete_argument(tool: &dyn RuntimeTool, caller: &Caller<'_>) -> bool {
    offers_athlete(tool.capabilities(), caller)
}

/// [`offers_athlete_argument`] over a tool's capabilities.
fn offers_athlete(capabilities: ToolCapabilities, caller: &Caller<'_>) -> bool {
    !caller.is_external()
        || (caller.role.coach && !capabilities.contains(ToolCapabilities::WRITES_DATA))
}

/// Narrow a built listing to what `caller` is offered: every tool
/// [`withheld`] withholds goes, and so does the `athlete` property of every
/// tool left that does not offer it ([`offers_athlete_argument`]).
pub fn retain_offered(registry: &ToolRegistry, schemas: &mut Vec<ToolSchema>, caller: &Caller<'_>) {
    schemas.retain(|schema| {
        registry
            .get(&schema.name)
            .is_none_or(|tool| withheld(tool.as_ref(), caller).is_none())
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
        }
    }

    #[test]
    fn the_coach_tools_go_externally_to_a_coach_and_an_admin_only() {
        let roster = tool(ToolReach::AnyTransport, Audience::Coach, READ);
        assert_eq!(
            decide(roster, &caller(ATHLETE, &READ_ONLY, Transport::McpHttp)),
            Some(Withheld::Role(Audience::Coach))
        );
        assert_eq!(
            decide(roster, &caller(COACH, &READ_ONLY, Transport::McpHttp)),
            None
        );
        assert_eq!(
            decide(roster, &caller(ADMIN, &READ_ONLY, Transport::A2a)),
            None
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
            None
        );
        assert_eq!(
            decide(agents, &caller(COACH, &with_admin, Transport::ApiKey)),
            Some(Withheld::Transport)
        );
        assert_eq!(
            decide(agents, &caller(ATHLETE, &READ_ONLY, Transport::MobileApp)),
            None
        );
    }

    #[test]
    fn an_app_only_tool_is_withheld_externally_from_everyone() {
        let publish = tool(ToolReach::AppOnly, Audience::Athlete, WRITE);
        let every = OAuthScope::self_grant();
        for role in [ATHLETE, COACH, ADMIN] {
            assert_eq!(
                decide(publish, &caller(role, &every, Transport::ApiKey)),
                Some(Withheld::Transport)
            );
            assert_eq!(
                decide(publish, &caller(role, &every, Transport::WebApp)),
                None
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
            Some(Withheld::Scope(OAuthScope::Admin))
        );
        let every = OAuthScope::self_grant();
        assert_eq!(
            decide(admin_tool, &caller(ADMIN, &every, Transport::McpHttp)),
            None
        );
        assert_eq!(
            decide(admin_tool, &caller(COACH, &every, Transport::WebApp)),
            Some(Withheld::Role(Audience::Admin))
        );
    }

    #[test]
    fn the_athlete_argument_goes_externally_to_a_coach_on_reads_alone() {
        let every = OAuthScope::self_grant();
        assert!(offers_athlete(
            READ,
            &caller(COACH, &every, Transport::McpHttp)
        ));
        assert!(!offers_athlete(
            WRITE,
            &caller(COACH, &every, Transport::McpHttp)
        ));
        assert!(!offers_athlete(
            READ,
            &caller(ADMIN, &every, Transport::McpHttp)
        ));
        assert!(!offers_athlete(
            READ,
            &caller(ATHLETE, &every, Transport::A2a)
        ));
        assert!(offers_athlete(
            WRITE,
            &caller(ATHLETE, &every, Transport::WebApp)
        ));
    }

    #[test]
    fn an_undeclared_transport_is_external() {
        let roster = tool(ToolReach::AnyTransport, Audience::Coach, READ);
        let undeclared = Caller {
            role: ATHLETE,
            grant: &READ_ONLY,
            transport: None,
        };
        assert_eq!(
            decide(roster, &undeclared),
            Some(Withheld::Role(Audience::Coach))
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
                                assert_ne!(answer, Some(Withheld::Transport));
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
}
