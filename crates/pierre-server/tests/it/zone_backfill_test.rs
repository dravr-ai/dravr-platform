// ABOUTME: The zone backfill re-cuts legacy stored zones with cageux's one model, once, and only in its tenant
// ABOUTME: Drives run_zone_backfill against a real database: dry run, apply, idempotent re-run, rows without inputs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use dravr_cageux::zones::{HrZones, PowerZones};
use pierre_core::models::zones::{HrZoneModel, HrZoneSet, PowerZoneModel, PowerZoneSet};
use pierre_core::models::{SportType, TenantId, UserPhysiologicalProfile};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::UserPhysiologicalProfileRepository;
use pierre_database::DatabaseProvider;
use pierre_services::zone_backfill::{run_zone_backfill, ZoneBackfillStats};
use pierre_test_support::db::create_test_db_with_key;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let db = create_test_db_with_key(b"test_encryption_key_32_bytes_long".to_vec())
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

/// A heart-rate set exactly as the platform stored it before the zones moved
/// to cageux: five upper bounds at 60/70/80/90 % of reserve, no model tag.
fn legacy_hr_zones() -> HrZoneSet {
    serde_json::from_str(r#"{"z1_max":134,"z2_max":148,"z3_max":162,"z4_max":176,"z5_max":190}"#)
        .unwrap()
}

/// A power set as the platform stored it: five FTP-percentage upper bounds.
fn legacy_power_zones() -> PowerZoneSet {
    serde_json::from_str(r#"{"z1_max":137,"z2_max":187,"z3_max":225,"z4_max":262,"z5_max":300}"#)
        .unwrap()
}

/// A profile carrying both legacy sets and the inputs to re-cut them.
fn legacy_profile(user_id: Uuid) -> UserPhysiologicalProfile {
    let mut profile = UserPhysiologicalProfile::new(user_id, SportType::Ride);
    profile.resting_hr = Some(50);
    profile.max_hr = Some(190);
    profile.ftp_watts = Some(250);
    profile.hr_zones = Some(legacy_hr_zones());
    profile.power_zones = Some(legacy_power_zones());
    profile
}

async fn stored(
    repo: &dyn UserPhysiologicalProfileRepository,
    tenant: TenantId,
    user_id: Uuid,
) -> UserPhysiologicalProfile {
    repo.get_user_physiological_profile(tenant, user_id)
        .await
        .unwrap()
        .expect("profile row")
}

#[tokio::test]
async fn a_legacy_row_still_reads_and_is_recut_once() {
    let db = make_test_db().await;
    let repos = db.repositories();
    let repo = repos.user_physiological_profile.as_ref();
    let tenant = TenantId::generate();
    let athlete = Uuid::new_v4();
    repo.upsert_user_physiological_profile(tenant, athlete, &legacy_profile(athlete))
        .await
        .unwrap();

    // The legacy row parses, tagged as such.
    let before = stored(repo, tenant, athlete).await;
    assert_eq!(before.hr_zones, Some(legacy_hr_zones()));
    assert_eq!(before.hr_zones.map(|z| z.model), Some(HrZoneModel::Legacy));
    assert_eq!(
        before.power_zones.map(|z| z.model),
        Some(PowerZoneModel::Legacy)
    );

    // A dry run counts the row and writes nothing.
    let dry = run_zone_backfill(repo, tenant, true).await.unwrap();
    assert_eq!(
        dry,
        ZoneBackfillStats {
            profiles_scanned: 1,
            rewritten: 1,
            already_current: 0,
            legacy_without_inputs: 0,
            dry_run: true,
        }
    );
    assert_eq!(
        stored(repo, tenant, athlete).await.hr_zones,
        Some(legacy_hr_zones())
    );

    // Applied, the row holds what cageux cuts from its own inputs.
    let applied = run_zone_backfill(repo, tenant, false).await.unwrap();
    assert_eq!(applied.rewritten, 1);
    let after = stored(repo, tenant, athlete).await;
    let expected_hr = HrZoneSet::from_zones(&HrZones::for_athlete(None, 50, 190).unwrap());
    let expected_power = PowerZoneSet::from_zones(&PowerZones::coggan(250.0).unwrap());
    assert_eq!(after.hr_zones, Some(expected_hr));
    assert_eq!(after.power_zones, Some(expected_power));
    // Resting 50, maximum 190 and no VO2max: Uth's ratio puts the athlete in
    // the advanced tier, which carries a zone 6 up to their maximum.
    assert_eq!(after.hr_zones.and_then(|z| z.z6_max), Some(190));
    assert_eq!(after.power_zones.and_then(|z| z.z6_max), Some(375));
    // Nothing but the zone copies moved.
    assert_eq!(after.resting_hr, Some(50));
    assert_eq!(after.max_hr, Some(190));
    assert_eq!(after.ftp_watts, Some(250));

    // Re-running is a no-op.
    let again = run_zone_backfill(repo, tenant, false).await.unwrap();
    assert_eq!(
        again,
        ZoneBackfillStats {
            profiles_scanned: 1,
            rewritten: 0,
            already_current: 1,
            legacy_without_inputs: 0,
            dry_run: false,
        }
    );
    assert_eq!(
        stored(repo, tenant, athlete).await.hr_zones,
        Some(expected_hr)
    );
}

#[tokio::test]
async fn a_legacy_set_without_its_inputs_is_kept_and_counted() {
    let db = make_test_db().await;
    let repos = db.repositories();
    let repo = repos.user_physiological_profile.as_ref();
    let tenant = TenantId::generate();
    let athlete = Uuid::new_v4();
    let mut profile = legacy_profile(athlete);
    // The heart rates are gone; the FTP is still there.
    profile.resting_hr = None;
    profile.max_hr = None;
    repo.upsert_user_physiological_profile(tenant, athlete, &profile)
        .await
        .unwrap();

    let stats = run_zone_backfill(repo, tenant, false).await.unwrap();
    assert_eq!(
        stats,
        ZoneBackfillStats {
            profiles_scanned: 1,
            rewritten: 1,
            already_current: 0,
            legacy_without_inputs: 1,
            dry_run: false,
        },
        "the power set still has its FTP; the heart-rate set has no inputs"
    );
    let after = stored(repo, tenant, athlete).await;
    assert_eq!(
        after.hr_zones,
        Some(legacy_hr_zones()),
        "no default physiology"
    );
    assert_eq!(
        after.power_zones.map(|z| z.model),
        Some(PowerZoneModel::Coggan)
    );

    // A re-run writes nothing, and the legacy heart-rate set it could not
    // re-cut is still reported as legacy, never as current.
    let again = run_zone_backfill(repo, tenant, false).await.unwrap();
    assert_eq!(
        again,
        ZoneBackfillStats {
            profiles_scanned: 1,
            rewritten: 0,
            already_current: 0,
            legacy_without_inputs: 1,
            dry_run: false,
        }
    );
}

#[tokio::test]
async fn the_backfill_stays_inside_its_tenant() {
    let db = make_test_db().await;
    let repos = db.repositories();
    let repo = repos.user_physiological_profile.as_ref();
    let (tenant_a, tenant_b) = (TenantId::generate(), TenantId::generate());
    let athlete = Uuid::new_v4();
    for tenant in [tenant_a, tenant_b] {
        repo.upsert_user_physiological_profile(tenant, athlete, &legacy_profile(athlete))
            .await
            .unwrap();
    }

    let stats = run_zone_backfill(repo, tenant_a, false).await.unwrap();
    assert_eq!(stats.profiles_scanned, 1);
    assert_eq!(
        stored(repo, tenant_a, athlete)
            .await
            .hr_zones
            .map(|z| z.model),
        Some(HrZoneModel::KarvonenVo2maxTiered)
    );
    assert_eq!(
        stored(repo, tenant_b, athlete).await.hr_zones,
        Some(legacy_hr_zones()),
        "another tenant's row is not touched"
    );
}
