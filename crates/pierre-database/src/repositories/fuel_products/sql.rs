// ABOUTME: The fuel-product bank's SQL, written once with $n placeholders for both backends
// ABOUTME: Catalogue reads, the review queue, the approval's supersede and relink, and the athlete's items (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// A catalogue product and its current version, aliased so one row decoder
/// reads it wherever it is joined: `p_*` for the product, `v_*` for the
/// version where the names would clash.
macro_rules! catalogue_columns {
    () => {
        r"
            p.id AS p_id, p.gtin, p.brand, p.name, p.variant, p.form,
            p.created_at AS p_created_at,
            v.id AS v_id, v.valid_from, v.superseded_by, v.serving_label, v.serving_g,
            v.energy_kcal, v.carbs_g, v.sugars_g, v.protein_g, v.fat_g, v.sodium_mg,
            v.potassium_mg, v.caffeine_mg, v.fluid_ml, v.carbs_g_per_100g,
            v.sugars_g_per_100g, v.protein_g_per_100g, v.fat_g_per_100g,
            v.sodium_mg_per_100g, v.salt_g_per_100g, v.glucose_fructose_ratio,
            v.provenance_json, v.approved_by_user_id, v.approved_at,
            v.created_at AS v_created_at
        "
    };
}

macro_rules! catalogue_select {
    () => {
        concat!(
            "SELECT ",
            catalogue_columns!(),
            r"
            FROM fuel_products p
            JOIN fuel_product_versions v
              ON v.product_id = p.id AND v.superseded_by IS NULL
            "
        )
    };
}

/// Products whose GTIN is `$1`, or whose natural key contains each of the
/// `LIKE` patterns `$2`..`$5` (an unused one is `%`), at most `$6`.
pub(crate) const SEARCH_PRODUCTS_SQL: &str = concat!(
    catalogue_select!(),
    r"
            WHERE p.gtin = $1
               OR (p.natural_key LIKE $2 ESCAPE '\'
                   AND p.natural_key LIKE $3 ESCAPE '\'
                   AND p.natural_key LIKE $4 ESCAPE '\'
                   AND p.natural_key LIKE $5 ESCAPE '\')
            ORDER BY p.natural_key
            LIMIT $6
            "
);

/// The live catalogue, `$1` rows from `$2`: each product with its current
/// version and how many versions it has had, in natural-key order.
pub(crate) const LIST_CATALOGUE_SQL: &str = concat!(
    "SELECT ",
    catalogue_columns!(),
    r",
            (SELECT COUNT(*) FROM fuel_product_versions h WHERE h.product_id = p.id)
                AS version_count
            FROM fuel_products p
            JOIN fuel_product_versions v
              ON v.product_id = p.id AND v.superseded_by IS NULL
            ORDER BY p.natural_key, p.id
            LIMIT $1 OFFSET $2
            "
);

/// One product by id `$1`.
pub(crate) const GET_PRODUCT_SQL: &str = concat!(catalogue_select!(), "WHERE p.id = $1");

/// One product by GTIN `$1`.
pub(crate) const GET_PRODUCT_BY_GTIN_SQL: &str = concat!(catalogue_select!(), "WHERE p.gtin = $1");

/// A proposal's columns, its photos reduced to how many are stored.
macro_rules! proposal_columns {
    () => {
        r"
            id, product_id, brand, name, variant, form, gtin, source_kind, source_url,
            (SELECT COUNT(*) FROM fuel_proposal_images img
              WHERE img.proposal_id = fuel_product_proposals.id) AS image_count,
            source_hash, facts_json, provenance_json, gate_flags_json, notes,
            ingredients_text, multiple_transportable_stated, status,
            proposed_by_user_id, reviewed_by_user_id, reviewed_at, reject_reason,
            reject_note, approved_version_id, created_at
        "
    };
}

/// Queue a proposal, status pending: `$1` id, `$2` product, `$3` brand, `$4`
/// name, `$5` variant, `$6` form, `$7` natural key, `$8` gtin, `$9` source
/// kind, `$10` source URL, `$11` source hash, `$12` facts, `$13` provenance,
/// `$14` gate flags, `$15` notes, `$16` ingredients, `$17` blend claim,
/// `$18` proposer, `$19` created at.
pub(crate) const INSERT_PROPOSAL_SQL: &str = r"
            INSERT INTO fuel_product_proposals (
                id, product_id, brand, name, variant, form, natural_key, gtin,
                source_kind, source_url, source_hash, facts_json, provenance_json,
                gate_flags_json, notes, ingredients_text, multiple_transportable_stated,
                proposed_by_user_id, status, created_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                    $17, $18, 'pending', $19)
            ";

/// Store photo `$2` (its place in the order sent) of proposal `$1`: `$3`
/// bytes, `$4` media type, `$5` SHA-256 hex.
pub(crate) const INSERT_PROPOSAL_IMAGE_SQL: &str = r"
            INSERT INTO fuel_proposal_images (proposal_id, position, bytes, mime, sha256)
            VALUES ($1, $2, $3, $4, $5)
            ";

/// The photos of proposal `$1` without their bytes, in the order sent.
pub(crate) const LIST_PROPOSAL_IMAGES_SQL: &str = r"
            SELECT CAST(position AS BIGINT) AS position, mime,
                   CAST(length(bytes) AS BIGINT) AS byte_len, sha256
            FROM fuel_proposal_images
            WHERE proposal_id = $1
            ORDER BY position
            ";

/// Photo `$2` of proposal `$1`.
pub(crate) const GET_PROPOSAL_IMAGE_SQL: &str = r"
            SELECT bytes, mime FROM fuel_proposal_images
            WHERE proposal_id = $1 AND position = $2
            ";

/// The oldest proposal pending or approved under natural key `$1` whose gate
/// flags do not match `$2` (the illegible flag, quoted and `LIKE`-wrapped):
/// an illegible filing is named by its listing, not its label, so it holds
/// no key.
pub(crate) const PROPOSAL_HOLDING_KEY_SQL: &str = r"
            SELECT id FROM fuel_product_proposals
            WHERE natural_key = $1 AND status IN ('pending', 'approved')
              AND gate_flags_json NOT LIKE $2
            ORDER BY created_at, id
            LIMIT 1
            ";

/// Close proposal `$1`, just queued by the ingestion job, as a duplicate at
/// `$2` with note `$3`. No reviewer: the job filed it already decided, so
/// its source hash is on file and the next run skips it unread.
pub(crate) const FILE_AS_DUPLICATE_SQL: &str = r"
            UPDATE fuel_product_proposals
            SET status = 'rejected', reviewed_at = $2, reject_reason = 'duplicate',
                reject_note = $3
            WHERE id = $1 AND status = 'pending'
            ";

/// Proposals in status `$1`, oldest first, `$2` of them after `$3`.
pub(crate) const LIST_PROPOSALS_SQL: &str = concat!(
    "SELECT ",
    proposal_columns!(),
    r"
            FROM fuel_product_proposals
            WHERE status = $1
            ORDER BY created_at, id
            LIMIT $2 OFFSET $3
            "
);

/// One proposal by id `$1`.
pub(crate) const GET_PROPOSAL_SQL: &str = concat!(
    "SELECT ",
    proposal_columns!(),
    " FROM fuel_product_proposals WHERE id = $1"
);

/// How many proposals carry source hash `$1`.
pub(crate) const COUNT_PROPOSALS_BY_HASH_SQL: &str =
    "SELECT COUNT(*) FROM fuel_product_proposals WHERE source_hash = $1";

/// Take pending proposal `$1` for approval by `$2` at `$3`. Zero rows means
/// it was no longer pending; on Postgres the row lock makes a concurrent
/// approval wait here and then find it approved.
pub(crate) const CLAIM_PROPOSAL_SQL: &str = r"
            UPDATE fuel_product_proposals
            SET status = 'approved', reviewed_by_user_id = $2, reviewed_at = $3
            WHERE id = $1 AND status = 'pending'
            ";

/// Lock pending proposal `$1` without changing it, for the length of an item
/// write. Zero rows means it was decided; on Postgres a concurrent approval's
/// claim waits for this transaction, and this one for the approval's.
pub(crate) const HOLD_PENDING_PROPOSAL_SQL: &str = r"
            UPDATE fuel_product_proposals
            SET status = status
            WHERE id = $1 AND status = 'pending'
            ";

/// The status of proposal `$1`.
pub(crate) const PROPOSAL_STATUS_SQL: &str =
    "SELECT status FROM fuel_product_proposals WHERE id = $1";

/// Record on approved proposal `$1` the version `$2` it wrote on product `$3`.
pub(crate) const RECORD_APPROVAL_SQL: &str = r"
            UPDATE fuel_product_proposals
            SET approved_version_id = $2, product_id = $3
            WHERE id = $1
            ";

/// Reject pending proposal `$1`: reviewer `$2`, at `$3`, reason `$4`, note `$5`.
pub(crate) const REJECT_PROPOSAL_SQL: &str = r"
            UPDATE fuel_product_proposals
            SET status = 'rejected', reviewed_by_user_id = $2, reviewed_at = $3,
                reject_reason = $4, reject_note = $5
            WHERE id = $1 AND status = 'pending'
            ";

/// A product's id and GTIN by id `$1`.
pub(crate) const PRODUCT_IDENTITY_BY_ID_SQL: &str =
    "SELECT id, gtin FROM fuel_products WHERE id = $1";

/// A product's id and GTIN by GTIN `$1`.
pub(crate) const PRODUCT_IDENTITY_BY_GTIN_SQL: &str =
    "SELECT id, gtin FROM fuel_products WHERE gtin = $1";

/// A product's id and GTIN by natural key `$1`.
pub(crate) const PRODUCT_IDENTITY_BY_KEY_SQL: &str =
    "SELECT id, gtin FROM fuel_products WHERE natural_key = $1";

/// Give product `$1`, which has none, the GTIN `$2`.
pub(crate) const SET_PRODUCT_GTIN_SQL: &str =
    "UPDATE fuel_products SET gtin = $2 WHERE id = $1 AND gtin IS NULL";

/// Enter a product: `$1` id, `$2` gtin, `$3` brand, `$4` name, `$5` variant,
/// `$6` form, `$7` natural key, `$8` when.
pub(crate) const INSERT_PRODUCT_SQL: &str = r"
            INSERT INTO fuel_products (id, gtin, brand, name, variant, form, natural_key, created_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ";

/// Point product `$1`'s current version at its successor `$2`. The foreign
/// key on `superseded_by` is checked at commit, after `$2` is written.
pub(crate) const SUPERSEDE_CURRENT_VERSION_SQL: &str = r"
            UPDATE fuel_product_versions
            SET superseded_by = $2
            WHERE product_id = $1 AND superseded_by IS NULL
            ";

/// Write a version: `$1` id, `$2` product, `$3` valid from, `$4` serving
/// label, `$5`..`$21` the numeric facts in column order, `$22` provenance,
/// `$23` approver, `$24` approved at (also the row's created at).
pub(crate) const INSERT_VERSION_SQL: &str = r"
            INSERT INTO fuel_product_versions (
                id, product_id, valid_from, superseded_by, serving_label, serving_g,
                energy_kcal, carbs_g, sugars_g, protein_g, fat_g, sodium_mg,
                potassium_mg, caffeine_mg, fluid_ml, carbs_g_per_100g,
                sugars_g_per_100g, protein_g_per_100g, fat_g_per_100g,
                sodium_mg_per_100g, salt_g_per_100g, glucose_fructose_ratio,
                provenance_json, approved_by_user_id, approved_at, created_at
            )
            VALUES ($1, $2, $3, NULL, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $24)
            ";

/// Hand product `$2` to every item that carried proposal `$1` while it was
/// pending — except a live item whose athlete already carries `$2`, which
/// `RETIRE_DUPLICATE_PENDING_ITEMS_SQL` closes next. Keyed by the proposal,
/// across tenants by design: the proposal is global, and the subquery keeps
/// each item to its own tenant and athlete.
pub(crate) const RELINK_PENDING_ITEMS_SQL: &str = r"
            UPDATE athlete_fuel_items
            SET product_id = $2
            WHERE proposal_id = $1 AND product_id IS NULL
              AND (removed_at IS NOT NULL OR NOT EXISTS (
                  SELECT 1 FROM athlete_fuel_items held
                  WHERE held.tenant_id = athlete_fuel_items.tenant_id
                    AND held.user_id = athlete_fuel_items.user_id
                    AND held.product_id = $2
                    AND held.removed_at IS NULL
              ))
            ";

/// The pending items `RELINK_PENDING_ITEMS_SQL` left: their athlete already
/// carries product `$2`, so each is linked to it and removed at `$3`.
pub(crate) const RETIRE_DUPLICATE_PENDING_ITEMS_SQL: &str = r"
            UPDATE athlete_fuel_items
            SET product_id = $2, removed_at = $3
            WHERE proposal_id = $1 AND product_id IS NULL
            ";

/// How many live items athlete `$2` carries in tenant `$1`.
pub(crate) const COUNT_LIVE_ITEMS_SQL: &str = r"
            SELECT COUNT(*) FROM athlete_fuel_items
            WHERE tenant_id = $1 AND user_id = $2 AND removed_at IS NULL
            ";

/// The live item of athlete `$2` in tenant `$1` that carries product `$3`.
pub(crate) const FIND_LIVE_PRODUCT_ITEM_SQL: &str = r"
            SELECT id FROM athlete_fuel_items
            WHERE tenant_id = $1 AND user_id = $2 AND product_id = $3 AND removed_at IS NULL
            ";

/// The live item of athlete `$2` in tenant `$1` that carries pending proposal `$3`.
pub(crate) const FIND_LIVE_PENDING_ITEM_SQL: &str = r"
            SELECT id FROM athlete_fuel_items
            WHERE tenant_id = $1 AND user_id = $2 AND proposal_id = $3
              AND product_id IS NULL AND removed_at IS NULL
            ";

/// The live item of athlete `$2` in tenant `$1` that carries a still-pending
/// filing with natural key `$3` and form `$4`.
pub(crate) const FIND_LIVE_PENDING_FILING_SQL: &str = r"
            SELECT i.id FROM athlete_fuel_items i
            JOIN fuel_product_proposals p ON p.id = i.proposal_id
            WHERE i.tenant_id = $1 AND i.user_id = $2 AND i.product_id IS NULL
              AND i.removed_at IS NULL AND p.status = 'pending'
              AND p.natural_key = $3 AND p.form = $4
            LIMIT 1
            ";

/// How many products have id `$1`.
pub(crate) const COUNT_PRODUCT_SQL: &str = "SELECT COUNT(*) FROM fuel_products WHERE id = $1";

/// Proposal `$1`'s status and the product an approval landed it on.
pub(crate) const PROPOSAL_LINK_SQL: &str =
    "SELECT status, product_id FROM fuel_product_proposals WHERE id = $1";

/// Carry an item: `$1` id, `$2` tenant, `$3` user, `$4` product, `$5`
/// proposal, `$6` note, `$7` when.
pub(crate) const INSERT_ITEM_SQL: &str = r"
            INSERT INTO athlete_fuel_items (
                id, tenant_id, user_id, product_id, proposal_id, note, created_at, removed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, NULL)
            ";

/// Athlete `$2`'s live items in tenant `$1`, oldest first, each with its
/// product's current version and its proposal when it has them.
pub(crate) const LIST_ITEMS_SQL: &str = concat!(
    r"
            SELECT i.id AS i_id, i.product_id AS i_product_id, i.proposal_id AS i_proposal_id,
                   i.note AS i_note, i.created_at AS i_created_at,
                   pr.brand AS pr_brand, pr.name AS pr_name, pr.variant AS pr_variant,
                   pr.form AS pr_form, pr.facts_json AS pr_facts_json, pr.status AS pr_status,
            ",
    catalogue_columns!(),
    r"
            FROM athlete_fuel_items i
            LEFT JOIN fuel_product_proposals pr ON pr.id = i.proposal_id
            LEFT JOIN fuel_products p ON p.id = i.product_id
            LEFT JOIN fuel_product_versions v
              ON v.product_id = p.id AND v.superseded_by IS NULL
            WHERE i.tenant_id = $1 AND i.user_id = $2 AND i.removed_at IS NULL
            ORDER BY i.created_at, i.id
            "
);

/// Stop carrying item `$3` of athlete `$2` in tenant `$1`, at `$4`.
pub(crate) const REMOVE_ITEM_SQL: &str = r"
            UPDATE athlete_fuel_items
            SET removed_at = $4
            WHERE tenant_id = $1 AND user_id = $2 AND id = $3 AND removed_at IS NULL
            ";
