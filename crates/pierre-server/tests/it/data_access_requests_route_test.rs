// ABOUTME: carnet#1646 — /api/user/data-access-requests: a request's lifecycle, its legal deadline and its letter per law and language
// ABOUTME: No other athlete and no other tenant reaches a request; it rides in the athlete's export and goes with an account reset

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The data access request routes, driven through the real auth router
//! against a real database. The deadline arithmetic is unit-tested beside
//! `pierre_services::data_access_requests::due_on`; this suite holds what the
//! athlete reaches over HTTP.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration as Wait;

use axum::Router;
use chrono::{Days, Months, Utc};
use serde_json::{json, Value};
use tokio::time;
use uuid::Uuid;

use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_database::repositories::AccountResetCommit;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;

use crate::common::create_test_server_resources;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

const BASE: &str = "/api/user/data-access-requests";

struct Setup {
    resources: Arc<ServerContext>,
    routes: Router,
}

impl Setup {
    async fn new() -> Self {
        let resources = create_test_server_resources().await.unwrap();
        Self {
            routes: AuthRoutes::routes(resources.auth_routes_context()),
            resources,
        }
    }

    fn repos(&self) -> Arc<RepositoryRegistry> {
        Arc::clone(&self.resources.common.repos)
    }

    fn token(&self, user: &User, tenant: Option<TenantId>) -> String {
        let jwt = self
            .resources
            .auth
            .auth_manager
            .generate_token_with_tenant(
                user,
                &self.resources.auth.jwks_manager,
                tenant.map(|t| t.to_string()),
            )
            .unwrap();
        format!("Bearer {jwt}")
    }

    async fn get(&self, token: &str, path: &str) -> AxumTestResponse {
        AxumTestRequest::get(path)
            .header("Authorization", token)
            .send(self.routes.clone())
            .await
    }

    async fn post(&self, token: &str, path: &str, body: &Value) -> AxumTestResponse {
        AxumTestRequest::post(path)
            .header("Authorization", token)
            .json(body)
            .send(self.routes.clone())
            .await
    }

    async fn patch(&self, token: &str, path: &str, body: &Value) -> AxumTestResponse {
        AxumTestRequest::patch(path)
            .header("Authorization", token)
            .json(body)
            .send(self.routes.clone())
            .await
    }

    async fn create(&self, token: &str, body: &Value) -> Value {
        let response = self.post(token, BASE, body).await;
        assert_eq!(response.status(), 201, "{}", response.body_text());
        response.json()
    }
}

async fn athlete(repos: &RepositoryRegistry, label: &str, name: &str) -> (User, TenantId) {
    let mut user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some(name.to_owned()),
    );
    user.user_status = UserStatus::Active;
    repos.users.create(&user).await.unwrap();
    let tenant = tenant_owned_by(repos, &user, label).await;
    (user, tenant)
}

async fn tenant_owned_by(repos: &RepositoryRegistry, user: &User, label: &str) -> TenantId {
    let tenant = Tenant::new(
        format!("{label} tenant"),
        format!("{label}-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    tenant.id
}

fn quebec_request() -> Value {
    json!({
        "controller": "Acme Fitness",
        "controller_contact": "privacy@acme.example",
        "athlete_identifier": "ana@example.com",
        "law": "quebec_p39_1",
        "scope": "access_portability",
        "letter_locale": "fr",
    })
}

fn gdpr_request() -> Value {
    json!({
        "controller": "Velo GmbH",
        "controller_contact": "Velo GmbH, Datenschutz, Hauptstraße 1, Berlin",
        "athlete_identifier": "ana@example.com",
        "law": "gdpr",
        "scope": "access",
        "letter_locale": "en",
    })
}

#[tokio::test]
async fn a_request_goes_from_draft_to_sent_to_answered() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana", "Ana Tremblay").await;
    let token = setup.token(&ana, Some(tenant));

    let created = setup.create(&token, &quebec_request()).await;
    assert_eq!(
        created["status"], "drafted",
        "bug: a new request is not a draft"
    );
    assert_eq!(
        created["derived_state"],
        Value::Null,
        "bug: a draft reads due soon or overdue"
    );
    assert_eq!(
        created["due_on"],
        Value::Null,
        "bug: a draft has a due date before it is sent"
    );
    let reference = created["reference"].as_str().unwrap();
    let year = Utc::now().format("%Y").to_string();
    assert!(
        reference.starts_with(&format!("DRV-{year}-")) && reference.len() == 15,
        "bug: the reference is not DRV-<year>-<6>: {reference}"
    );
    let id = created["id"].as_str().unwrap();
    let path = format!("{BASE}/{id}");

    let edited = setup
        .patch(
            &token,
            &path,
            &json!({ "controller": "  Acme Fitness Inc.  " }),
        )
        .await;
    assert_eq!(edited.status(), 200, "{}", edited.body_text());
    assert_eq!(
        edited.json::<Value>()["controller"],
        "Acme Fitness Inc.",
        "bug: the draft edit was not trimmed and kept"
    );

    // Sent ten days ago: Quebec's 30 days leave 20.
    let today = Utc::now().date_naive();
    let sent_on = today.checked_sub_days(Days::new(10)).unwrap();
    let sent = setup
        .post(
            &token,
            &format!("{path}/sent"),
            &json!({ "sent_on": sent_on }),
        )
        .await;
    assert_eq!(sent.status(), 200, "{}", sent.body_text());
    let sent: Value = sent.json();
    assert_eq!(sent["status"], "sent", "bug: the request is not sent");
    assert_eq!(
        sent["due_on"],
        json!(sent_on.checked_add_days(Days::new(30)).unwrap()),
        "bug: a Quebec request is not due 30 days after it was sent"
    );
    assert_eq!(
        sent["derived_state"],
        Value::Null,
        "bug: twenty days left reads due soon or overdue"
    );

    let draft_edit = setup
        .patch(&token, &path, &json!({ "controller": "Too late" }))
        .await;
    assert_eq!(
        draft_edit.status(),
        409,
        "bug: a sent request was edited as a draft: {}",
        draft_edit.body_text()
    );
    let extension = setup
        .post(
            &token,
            &format!("{path}/extension"),
            &json!({ "extended_until": today }),
        )
        .await;
    assert_eq!(
        extension.status(),
        400,
        "bug: a Quebec request took a GDPR extension: {}",
        extension.body_text()
    );

    let answered = setup
        .post(
            &token,
            &format!("{path}/answered"),
            &json!({ "answered_on": today }),
        )
        .await;
    assert_eq!(answered.status(), 200, "{}", answered.body_text());
    assert_eq!(
        answered.json::<Value>()["status"],
        "answered",
        "bug: the request is not answered"
    );
    let closed = setup
        .post(&token, &format!("{path}/close"), &json!({}))
        .await;
    assert_eq!(
        closed.status(),
        409,
        "bug: an answered request was closed: {}",
        closed.body_text()
    );

    let list = setup.get(&token, BASE).await;
    assert_eq!(list.status(), 200, "{}", list.body_text());
    let list: Value = list.json();
    assert_eq!(
        list["requests"].as_array().unwrap().len(),
        1,
        "bug: the list is not the athlete's one request: {list}"
    );
    assert_eq!(
        list["requests"][0]["reference"], reference,
        "bug: the listed request is not the one written"
    );
}

#[tokio::test]
async fn a_gdpr_request_reads_overdue_then_takes_an_extension() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana", "Ana Tremblay").await;
    let token = setup.token(&ana, Some(tenant));
    let created = setup.create(&token, &gdpr_request()).await;
    let path = format!("{BASE}/{}", created["id"].as_str().unwrap());

    let today = Utc::now().date_naive();
    let sent_on = today
        .checked_sub_months(Months::new(1))
        .unwrap()
        .checked_sub_days(Days::new(2))
        .unwrap();
    let sent: Value = setup
        .post(
            &token,
            &format!("{path}/sent"),
            &json!({ "sent_on": sent_on }),
        )
        .await
        .json();
    assert_eq!(
        sent["due_on"],
        json!(sent_on.checked_add_months(Months::new(1)).unwrap()),
        "bug: a GDPR request is not due one calendar month after it was sent"
    );
    assert_eq!(
        sent["derived_state"], "overdue",
        "bug: a request past its due date does not read overdue"
    );

    let too_far = sent_on.checked_add_months(Months::new(3)).unwrap();
    let refused = setup
        .post(
            &token,
            &format!("{path}/extension"),
            &json!({ "extended_until": too_far.checked_add_days(Days::new(1)).unwrap() }),
        )
        .await;
    assert_eq!(
        refused.status(),
        400,
        "bug: an extension past three months was taken: {}",
        refused.body_text()
    );
    let extended = setup
        .post(
            &token,
            &format!("{path}/extension"),
            &json!({ "extended_until": too_far }),
        )
        .await;
    assert_eq!(extended.status(), 200, "{}", extended.body_text());
    let extended: Value = extended.json();
    assert_eq!(
        extended["due_on"],
        json!(too_far),
        "bug: the extension did not become the due date"
    );
    assert_eq!(
        extended["extended_until"],
        json!(too_far),
        "bug: the extension was not kept"
    );
    assert_eq!(
        extended["derived_state"],
        Value::Null,
        "bug: an extended request still reads overdue"
    );

    let closed = setup
        .post(&token, &format!("{path}/close"), &json!({}))
        .await;
    assert_eq!(closed.status(), 200, "{}", closed.body_text());
    assert_eq!(
        closed.json::<Value>()["status"],
        "closed",
        "bug: the request is not closed"
    );
}

#[tokio::test]
async fn the_letter_follows_its_law_scope_and_language() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana", "Ana Tremblay").await;
    let token = setup.token(&ana, Some(tenant));

    let quebec = setup.create(&token, &quebec_request()).await;
    let reference = quebec["reference"].as_str().unwrap();
    let letter: Value = setup
        .get(
            &token,
            &format!("{BASE}/{}/letter", quebec["id"].as_str().unwrap()),
        )
        .await
        .json();
    assert_eq!(
        letter["locale"], "fr",
        "bug: the letter is not in the request's own language"
    );
    let body = letter["body"].as_str().unwrap();
    let subject = letter["subject"].as_str().unwrap();
    assert!(
        subject.contains(reference),
        "bug: the subject lacks the reference: {subject}"
    );
    assert!(
        body.contains("P-39.1") && body.contains("Acme Fitness"),
        "bug: the Quebec letter does not cite the Act or name the company: {body}"
    );
    assert!(
        body.contains("ana@example.com") && body.contains(reference),
        "bug: the letter lacks the identifier or the reference: {body}"
    );
    assert!(
        body.trim_end().ends_with("Ana Tremblay"),
        "bug: the letter is not signed with the athlete's name: {body}"
    );
    let paragraphs = body.split("\n\n").count();
    assert_eq!(
        paragraphs, 10,
        "bug: a Quebec access + portability letter is ten paragraphs: {body}"
    );
    assert_eq!(
        letter["text"],
        format!("{subject}\n\n{body}\n"),
        "bug: the text file is not the subject, a blank line and the body"
    );
    assert_eq!(
        letter["filename"],
        format!("{reference}.txt"),
        "bug: the text file is not named after the reference"
    );
    assert!(
        letter["mailto"]
            .as_str()
            .unwrap()
            .starts_with("mailto:privacy@acme.example?subject="),
        "bug: an email address gets no mailto link: {letter}"
    );

    // Asked for in English, the same request cites the Act word for word.
    let english: Value = setup
        .get(
            &token,
            &format!("{BASE}/{}/letter?locale=en", quebec["id"].as_str().unwrap()),
        )
        .await
        .json();
    let english_body = english["body"].as_str().unwrap();
    assert_eq!(
        english["locale"], "en",
        "bug: the asked-for language was not used"
    );
    assert!(
        english_body.contains(
            "Loi sur la protection des renseignements personnels dans le secteur privé (RLRQ, c. P-39.1), art. 27"
        ),
        "bug: the English letter does not cite the Act in French: {english_body}"
    );
    assert_ne!(english_body, body, "bug: the locale was ignored");

    let gdpr = setup.create(&token, &gdpr_request()).await;
    let letter: Value = setup
        .get(
            &token,
            &format!("{BASE}/{}/letter", gdpr["id"].as_str().unwrap()),
        )
        .await
        .json();
    let body = letter["body"].as_str().unwrap();
    assert_eq!(
        letter["locale"], "en",
        "bug: the GDPR letter is not in its request's language"
    );
    assert!(
        body.contains("Article 15") && body.contains("Regulation (EU) 2016/679"),
        "bug: the GDPR letter does not cite Art. 15: {body}"
    );
    assert!(
        !body.contains("Article 20"),
        "bug: an access-only letter asks for portability: {body}"
    );
    assert_eq!(
        body.split("\n\n").count(),
        9,
        "bug: an access-only letter is nine paragraphs: {body}"
    );
    assert_eq!(
        letter["mailto"],
        Value::Null,
        "bug: a postal address got a mailto link"
    );

    let unsupported = setup
        .get(
            &token,
            &format!("{BASE}/{}/letter?locale=it", gdpr["id"].as_str().unwrap()),
        )
        .await;
    assert_eq!(unsupported.status(), 400, "{}", unsupported.body_text());
}

#[tokio::test]
async fn another_athlete_never_reaches_a_request() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, ana_tenant) = athlete(&repos, "ana", "Ana").await;
    let (ben, ben_tenant) = athlete(&repos, "ben", "Ben").await;
    let ana_token = setup.token(&ana, Some(ana_tenant));
    let ben_token = setup.token(&ben, Some(ben_tenant));
    let created = setup.create(&ana_token, &quebec_request()).await;
    let path = format!("{BASE}/{}", created["id"].as_str().unwrap());

    for response in [
        setup.get(&ben_token, &path).await,
        setup.get(&ben_token, &format!("{path}/letter")).await,
        setup
            .patch(&ben_token, &path, &json!({ "controller": "Ben" }))
            .await,
        setup
            .post(
                &ben_token,
                &format!("{path}/sent"),
                &json!({ "sent_on": Utc::now().date_naive() }),
            )
            .await,
        setup
            .post(&ben_token, &format!("{path}/close"), &json!({}))
            .await,
    ] {
        assert_eq!(
            response.status(),
            404,
            "bug: another athlete reached Ana's request: {}",
            response.body_text()
        );
    }
    let list: Value = setup.get(&ben_token, BASE).await.json();
    assert_eq!(
        list["requests"],
        json!([]),
        "bug: another athlete lists Ana's request"
    );
    let still: Value = setup.get(&ana_token, &path).await.json();
    assert_eq!(still["status"], "drafted", "bug: a refused call changed it");
    assert_eq!(
        still["controller"], "Acme Fitness",
        "bug: another athlete's edit landed"
    );
}

#[tokio::test]
async fn a_request_stays_in_the_tenant_it_was_written_in() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, first) = athlete(&repos, "ana", "Ana").await;
    let second = tenant_owned_by(&repos, &ana, "ana-second").await;
    let in_first = setup.token(&ana, Some(first));
    let in_second = setup.token(&ana, Some(second));
    let created = setup.create(&in_first, &quebec_request()).await;
    let path = format!("{BASE}/{}", created["id"].as_str().unwrap());

    let read = setup.get(&in_second, &path).await;
    assert_eq!(
        read.status(),
        404,
        "bug: a request was read from another tenant: {}",
        read.body_text()
    );
    let close = setup
        .post(&in_second, &format!("{path}/close"), &json!({}))
        .await;
    assert_eq!(close.status(), 404, "{}", close.body_text());
    let list: Value = setup.get(&in_second, BASE).await.json();
    assert_eq!(list["requests"], json!([]), "bug: another tenant lists it");
    assert_eq!(
        setup.get(&in_first, &path).await.status(),
        200,
        "bug: the request is not read in the tenant it was written in"
    );
}

#[tokio::test]
async fn a_request_is_refused_without_the_athletes_own_session_or_with_bad_input() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana", "Ana").await;
    let token = setup.token(&ana, Some(tenant));

    let anonymous = AxumTestRequest::get(BASE).send(setup.routes.clone()).await;
    assert_eq!(anonymous.status(), 401, "{}", anonymous.body_text());

    let mut no_company = quebec_request();
    no_company["controller"] = json!("   ");
    assert_eq!(
        setup.post(&token, BASE, &no_company).await.status(),
        400,
        "bug: a request with no company was written"
    );
    let mut bad_law = quebec_request();
    bad_law["law"] = json!("ccpa");
    assert_eq!(
        setup.post(&token, BASE, &bad_law).await.status(),
        400,
        "bug: a request under an unknown law was written"
    );

    // With no language given, the letter takes the athlete's (French, the
    // default).
    let mut unlocalized = quebec_request();
    unlocalized.as_object_mut().unwrap().remove("letter_locale");
    assert_eq!(
        setup.create(&token, &unlocalized).await["letter_locale"],
        "fr",
        "bug: a request with no language did not take the athlete's"
    );

    let created = setup.create(&token, &quebec_request()).await;
    let future = Utc::now()
        .date_naive()
        .checked_add_days(Days::new(1))
        .unwrap();
    let response = setup
        .post(
            &token,
            &format!("{BASE}/{}/sent", created["id"].as_str().unwrap()),
            &json!({ "sent_on": future }),
        )
        .await;
    assert_eq!(
        response.status(),
        400,
        "bug: a sending date in the future was taken: {}",
        response.body_text()
    );
}

#[tokio::test]
async fn a_request_rides_in_the_export_and_goes_with_an_account_reset() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana", "Ana").await;
    let token = setup.token(&ana, Some(tenant));
    let created = setup.create(&token, &quebec_request()).await;

    // The export writes a few documents at once, process-wide, and the
    // suites run theirs in parallel: a busy refusal is retried, as it asks.
    let mut export = None;
    for _ in 0..200 {
        let response = setup.get(&token, "/api/user/export").await;
        if !response.body_text().contains("\"export_busy\"") {
            export = Some(response);
            break;
        }
        time::sleep(Wait::from_millis(50)).await;
    }
    let export = export.expect("the export stayed busy");
    assert_eq!(export.status(), 200, "{}", export.body_text());
    let doc: Value = export.json();
    let rows = doc["tables"]["data_access_requests"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        1,
        "bug: the athlete's request is not in their export"
    );
    assert_eq!(
        rows[0]["reference"], created["reference"],
        "bug: the exported row is not the athlete's request"
    );
    assert_eq!(
        rows[0]["controller_contact"], "privacy@acme.example",
        "bug: the export left out the company's contact"
    );

    let committed = repos.account_reset.reset_account(ana.id).await.unwrap();
    assert!(
        matches!(committed, AccountResetCommit::Reset(_)),
        "bug: the reset did not commit: {committed:?}"
    );
    let list: Value = setup.get(&token, BASE).await.json();
    assert_eq!(
        list["requests"],
        json!([]),
        "bug: an account reset left the athlete's requests"
    );
}
