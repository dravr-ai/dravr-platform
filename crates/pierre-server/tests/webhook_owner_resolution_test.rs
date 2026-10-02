// ABOUTME: Whose data a provider push event is about — the token owner, or the member a coach's confirmed link names
// ABOUTME: Runs the real resolver on the factory's database, so proposed, revoked and unbacked links are proven to resolve to nobody
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Webhook owner resolution, below the HTTP layer.
//!
//! A coach platform's push event names the athlete while the token belongs to
//! the coach. These tests seed the rows a real deployment holds — a token with
//! its `provider_user_id`, a group, its coach, members and their links — and
//! ask the resolver the webhook routes call whom each provider-side id names.

mod common;

use chrono::Utc;
use common::{create_test_database, create_test_user_with_plan};
use pierre_core::models::{
    DelegatedConnection, DelegationEndReason, RosterAthlete, TenantId, UserOAuthToken,
};
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_services::webhook_owner::{resolve_webhook_owner, CoachConnection, WebhookOwner};
use pierre_test_support::delegation::{add_group_member, create_coached_group, create_group_agent};
use std::sync::Arc;
use uuid::Uuid;

/// A coach platform: its push events name the athlete, its token is the coach's.
const PROVIDER: &str = "nolio";

struct Fixture {
    db: Arc<Database>,
    repos: Arc<RepositoryRegistry>,
    owner: Uuid,
    owner_tenant: TenantId,
    coach: Uuid,
    coach_tenant: TenantId,
    agent_id: String,
}

/// A group owner (on a plan with groups), a human coach in their own tenant,
/// and an agent persona the groups run.
async fn fixture() -> Fixture {
    let db = create_test_database().await.unwrap();
    let (owner, _, owner_tenant) =
        create_test_user_with_plan(&db, "owner@webhook-owner.test", "professional")
            .await
            .unwrap();
    let (coach, _, coach_tenant) =
        create_test_user_with_plan(&db, "coach@webhook-owner.test", "starter")
            .await
            .unwrap();
    let repos = Arc::clone(db.repositories());
    let agent_id = create_group_agent(&repos, owner, owner_tenant)
        .await
        .unwrap();
    Fixture {
        db,
        repos,
        owner,
        owner_tenant,
        coach,
        coach_tenant,
        agent_id,
    }
}

impl Fixture {
    /// An active group `coach` coaches.
    async fn group_coached_by(&self, name: &str, coach: Uuid) -> Uuid {
        create_coached_group(
            &self.repos,
            self.owner,
            self.owner_tenant,
            &self.agent_id,
            name,
            coach,
        )
        .await
        .unwrap()
    }

    /// An active group the fixture's coach coaches.
    async fn group(&self, name: &str) -> Uuid {
        self.group_coached_by(name, self.coach).await
    }

    /// An active member of `group` whose Dravr email is verified, so a roster
    /// athlete carrying that email binds to them.
    async fn member(&self, email: &str, group: Uuid) -> (Uuid, TenantId) {
        let (user, tenant) = self.unverified_member(email, group).await;
        self.repos
            .email_verification
            .mark_verified(user)
            .await
            .unwrap();
        (user, tenant)
    }

    /// An active member of `group` who never verified their Dravr email.
    async fn unverified_member(&self, email: &str, group: Uuid) -> (Uuid, TenantId) {
        let (user, _, tenant) = create_test_user_with_plan(&self.db, email, "starter")
            .await
            .unwrap();
        add_group_member(&self.repos, group, user, tenant)
            .await
            .unwrap();
        (user, tenant)
    }

    /// `coach` proposes linking roster athlete `athlete`, whose provider email
    /// is `athlete_email`, to `member`.
    async fn proposed_naming(
        &self,
        coach: (Uuid, TenantId),
        group: Uuid,
        member: Uuid,
        athlete: &str,
        athlete_email: Option<String>,
    ) -> DelegatedConnection {
        let proposal = DelegatedConnection::propose(
            PROVIDER.to_owned(),
            group,
            coach.0,
            coach.1,
            member,
            RosterAthlete {
                id: athlete.to_owned(),
                name: None,
                email: athlete_email,
            },
        );
        self.repos
            .delegated_connections
            .propose(&proposal)
            .await
            .unwrap()
            .expect("no live link holds this member or athlete yet")
    }

    /// `coach` proposes linking roster athlete `athlete` to `member`, the
    /// provider listing the athlete under the member's own Dravr email.
    async fn proposed_by(
        &self,
        coach: (Uuid, TenantId),
        group: Uuid,
        member: Uuid,
        athlete: &str,
    ) -> DelegatedConnection {
        let member_email = self
            .repos
            .users
            .get_global(member)
            .await
            .unwrap()
            .expect("the member exists")
            .email;
        self.proposed_naming(coach, group, member, athlete, Some(member_email))
            .await
    }

    /// The fixture's coach proposes the link.
    async fn proposed(&self, group: Uuid, member: Uuid, athlete: &str) -> DelegatedConnection {
        self.proposed_by((self.coach, self.coach_tenant), group, member, athlete)
            .await
    }

    /// The member confirms `link` in their own tenant.
    async fn confirm(&self, link: &DelegatedConnection, member_tenant: TenantId) {
        self.repos
            .delegated_connections
            .confirm(link.id, link.member_user_id, member_tenant, Utc::now())
            .await
            .unwrap()
            .expect("the member confirms their own proposal");
    }

    /// `user` connected `provider` themselves; the provider knows them as `id`.
    async fn own_token(&self, user: Uuid, tenant: TenantId, provider: &str, id: &str) {
        let mut token = UserOAuthToken::new(
            user,
            tenant.to_string(),
            provider.to_owned(),
            "access".to_owned(),
            Some("refresh".to_owned()),
            None,
            None,
        );
        token.provider_user_id = Some(id.to_owned());
        self.repos.oauth_tokens.upsert_token(&token).await.unwrap();
    }

    async fn resolve(&self, provider: &str, id: &str) -> Option<WebhookOwner> {
        resolve_webhook_owner(&self.repos, provider, id)
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_token_owner_is_resolved_directly() {
    let f = fixture().await;
    let (athlete, _, tenant) =
        create_test_user_with_plan(&f.db, "direct@webhook-owner.test", "starter")
            .await
            .unwrap();
    f.own_token(athlete, tenant, "strava", "4242").await;

    assert_eq!(
        f.resolve("strava", "4242").await,
        Some(WebhookOwner {
            user_id: athlete,
            tenant_id: tenant.to_string(),
            read_through: None,
        }),
        "the id a token carries names its owner, read with their own token"
    );
    assert_eq!(
        f.resolve("whoop", "4242").await,
        None,
        "the same id under another provider names nobody"
    );
}

#[tokio::test]
async fn a_token_owner_wins_over_a_confirmed_link_naming_the_same_id() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, member_tenant) = f.member("linked@webhook-owner.test", group).await;
    let link = f.proposed(group, member, "7001").await;
    f.confirm(&link, member_tenant).await;
    let (athlete, _, tenant) =
        create_test_user_with_plan(&f.db, "own-login@webhook-owner.test", "starter")
            .await
            .unwrap();
    f.own_token(athlete, tenant, PROVIDER, "7001").await;

    let owner = f.resolve(PROVIDER, "7001").await.unwrap();
    assert_eq!(owner.user_id, athlete);
    assert_eq!(owner.tenant_id, tenant.to_string());
    assert_eq!(owner.read_through, None);
}

#[tokio::test]
async fn a_confirmed_member_is_resolved_with_the_coach_connection_to_read_through() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, member_tenant) = f.member("member@webhook-owner.test", group).await;
    let link = f.proposed(group, member, "7002").await;
    f.confirm(&link, member_tenant).await;

    assert_eq!(
        f.resolve(PROVIDER, "7002").await,
        Some(WebhookOwner {
            user_id: member,
            tenant_id: member_tenant.to_string(),
            read_through: Some(CoachConnection {
                link_id: link.id,
                coach_user_id: f.coach,
                coach_tenant_id: f.coach_tenant,
            }),
        }),
        "the athlete has no token: the event lands on the member, in the tenant they \
         confirmed in, read with the coach's connection"
    );
    assert_eq!(
        f.resolve("strava", "7002").await,
        None,
        "a link is followed only for the provider it was made for"
    );
}

#[tokio::test]
async fn an_unconfirmed_member_is_dropped() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, _) = f.member("pending@webhook-owner.test", group).await;
    f.proposed(group, member, "7003").await;

    assert_eq!(
        f.resolve(PROVIDER, "7003").await,
        None,
        "a proposal is not consent: the member has not answered"
    );
}

#[tokio::test]
async fn a_revoked_member_is_dropped() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, member_tenant) = f.member("revoked@webhook-owner.test", group).await;
    let link = f.proposed(group, member, "7004").await;
    f.confirm(&link, member_tenant).await;
    assert!(
        f.resolve(PROVIDER, "7004").await.is_some(),
        "confirmed, the link resolves — so the drop below is the revoke's doing"
    );

    f.repos
        .delegated_connections
        .end_one(
            link.id,
            member,
            Some(member),
            DelegationEndReason::RevokedByMember,
            Utc::now(),
        )
        .await
        .unwrap()
        .expect("the member ends their confirmed link");

    assert_eq!(f.resolve(PROVIDER, "7004").await, None);
}

#[tokio::test]
async fn an_unknown_id_is_dropped() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, member_tenant) = f.member("other@webhook-owner.test", group).await;
    let link = f.proposed(group, member, "7005").await;
    f.confirm(&link, member_tenant).await;

    assert_eq!(
        f.resolve(PROVIDER, "9999").await,
        None,
        "an id no token and no link carries names nobody, whoever else is linked"
    );
}

#[tokio::test]
async fn a_confirmed_link_the_group_no_longer_backs_is_dropped() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (member, member_tenant) = f.member("left@webhook-owner.test", group).await;
    let link = f.proposed(group, member, "7006").await;
    f.confirm(&link, member_tenant).await;
    assert!(f.resolve(PROVIDER, "7006").await.is_some());

    // The membership ends without the link's eager end: the row still says
    // `confirmed`, and the resolver must fail closed on the relation.
    assert!(f
        .repos
        .groups
        .remove_member(&group.to_string(), member)
        .await
        .unwrap());

    assert_eq!(f.resolve(PROVIDER, "7006").await, None);
}

#[tokio::test]
async fn a_confirmed_link_whose_member_does_not_bind_by_email_is_dropped() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let coach = (f.coach, f.coach_tenant);

    // The member's Dravr email proves no one until it is verified: the read
    // path refuses this link, so the event names nobody.
    let (unverified, unverified_tenant) = f
        .unverified_member("unverified@webhook-owner.test", group)
        .await;
    let link = f.proposed(group, unverified, "7008").await;
    f.confirm(&link, unverified_tenant).await;
    assert_eq!(
        f.resolve(PROVIDER, "7008").await,
        None,
        "confirmed and group-backed, but the member's email is unverified"
    );
    f.repos
        .email_verification
        .mark_verified(unverified)
        .await
        .unwrap();
    assert_eq!(
        f.resolve(PROVIDER, "7008").await.map(|owner| owner.user_id),
        Some(unverified),
        "once the email binds the same link resolves — so the drop above was the binding's doing"
    );

    // The provider lists the roster athlete under someone else's email.
    let (other, other_tenant) = f.member("other@webhook-owner.test", group).await;
    let mismatched = f
        .proposed_naming(
            coach,
            group,
            other,
            "7009",
            Some("someone-else@webhook-owner.test".to_owned()),
        )
        .await;
    f.confirm(&mismatched, other_tenant).await;
    assert_eq!(f.resolve(PROVIDER, "7009").await, None);

    // A link that stored no athlete email, as one confirmed before links
    // kept it, binds no one either.
    let (legacy, legacy_tenant) = f.member("legacy@webhook-owner.test", group).await;
    let unnamed = f.proposed_naming(coach, group, legacy, "7010", None).await;
    f.confirm(&unnamed, legacy_tenant).await;
    assert_eq!(f.resolve(PROVIDER, "7010").await, None);
}

#[tokio::test]
async fn an_athlete_confirmed_under_two_coaches_is_dropped_rather_than_guessed() {
    let f = fixture().await;
    let group = f.group("Squad").await;
    let (first, first_tenant) = f.member("first@webhook-owner.test", group).await;
    let link = f.proposed(group, first, "7007").await;
    f.confirm(&link, first_tenant).await;

    let (other_coach, _, other_coach_tenant) =
        create_test_user_with_plan(&f.db, "coach2@webhook-owner.test", "starter")
            .await
            .unwrap();
    let other_group = f.group_coached_by("Other squad", other_coach).await;
    let (second, second_tenant) = f.member("second@webhook-owner.test", other_group).await;
    let other_link = f
        .proposed_by(
            (other_coach, other_coach_tenant),
            other_group,
            second,
            "7007",
        )
        .await;
    f.confirm(&other_link, second_tenant).await;

    assert_eq!(
        f.resolve(PROVIDER, "7007").await,
        None,
        "one athlete id confirmed for two members: the event is not handed to either"
    );
}
