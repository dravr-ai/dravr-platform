// ABOUTME: `pierre-cli fuel-ingest` — files fuel-product proposals from brands' Shopify storefronts, labels read by OCR
// ABOUTME: Checked-in brand list, discovery, dedupe by source hash, the embacle label read, the gate, a counted summary (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `pierre-cli fuel-ingest [--brand <slug>]… [--limit <n>] [--dry-run]`.
//!
//! The weekly `*-fuel-ingest-job` Cloud Run job runs it. For each brand in
//! [`BRANDS`] it lists the storefront's `/products.json`, keeps the fuel
//! packages, splits each into the flavours its photos show, downloads the last
//! [`shopify::MAX_IMAGES_PER_CANDIDATE`] photos, and files one proposal per
//! package the label could be read for. Nothing enters the catalogue here: a
//! proposal waits for an operator's approval.
//!
//! - **Dedupe before the LLM.** The source hash is taken over the page URL and
//!   every photo sent, in order; a hash already on file skips the package
//!   without a call, so a weekly rerun costs only the downloads.
//! - **The label is the only source.** The read goes through the configured
//!   chain with the contremaitre `fuel_panel_extraction` prompt; an answer
//!   from a tier that does not read images is discarded
//!   ([`extraction::read_label`]).
//! - **Flagged, never clamped.** The gate's violation codes travel on the
//!   proposal so the reviewer sees why; an illegible label is skipped.
//! - **Identity.** The brand is the table's display name, so one brand is one
//!   spelling; name and variant are what the label prints, falling back to
//!   the listing title and its flavour.
//! - **The stored photo** is the last one sent (the panel is usually last);
//!   the page URL on the proposal opens all of them.
//!
//! The run fails (non-zero exit, which the job-failure alert counts) when
//! labels were sent and not one read came back, or when no brand could be
//! listed.

mod extraction;
mod shopify;

use std::sync::Arc;

use pierre_contremaitre::registry::PromptRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    proposal_source_hash, NewFuelProposal, ProposalSource, ProposalSourceImage,
};
use pierre_core::redaction::redact_url;
use pierre_database::backends::factory::Database;
use pierre_database::RepositoryRegistry;
use pierre_llm::config::LlmChain;
use pierre_llm::ChatProvider;
use serde_json::json;
use tracing::{error, info, warn};

use extraction::{label_request, map_reading, read_label, ReadFailure, Reading};
use shopify::{
    download_image, fetch_fuel_products, label_candidates, LabelCandidate, LabelImage,
    ShopifyProduct,
};

/// A brand the job reads, and how its storefront is filtered to fuel.
#[derive(Debug)]
pub(crate) struct Brand {
    /// The `--brand` value.
    pub slug: &'static str,
    /// The brand as the catalogue names it.
    pub display: &'static str,
    /// The storefront base URL; `/products.json` answers under it.
    pub store: &'static str,
    /// Whole-word keywords, any of which in a listing's type, tags or title
    /// marks it as fuel (the shared exclusions in [`shopify`] still apply).
    pub keywords: &'static [&'static str],
}

/// The brands the job reads. Each store was checked to be a Shopify
/// storefront serving `/products.json` on 2026-10-29.
pub(crate) const BRANDS: [Brand; 4] = [
    Brand {
        slug: "naak",
        display: "Näak",
        store: "https://naak.com",
        keywords: &[
            "gel",
            "gels",
            "drink mix",
            "drink mixes",
            "bar",
            "bars",
            "waffle",
            "waffles",
            "puree",
            "purees",
        ],
    },
    Brand {
        slug: "tailwind",
        display: "Tailwind Nutrition",
        store: "https://tailwindnutrition.com",
        keywords: &["endurance fuel", "high carb fuel", "rapid hydration"],
    },
    Brand {
        slug: "skratch",
        display: "Skratch Labs",
        store: "https://www.skratchlabs.com",
        keywords: &[
            "drink mix",
            "energy chews",
            "energy bar",
            "sport fuel",
            "electrolyte",
        ],
    },
    Brand {
        slug: "gu",
        display: "GU Energy Labs",
        store: "https://guenergy.com",
        keywords: &[
            "gel",
            "energy gel",
            "liquid energy",
            "chews",
            "drink mix",
            "hydration tabs",
            "drink tabs",
            "capsules",
            "waffle",
        ],
    },
];

/// `pierre-cli fuel-ingest` arguments.
#[derive(clap::Args)]
pub(crate) struct FuelIngestArgs {
    /// A brand slug to read (repeatable); every brand when omitted.
    #[arg(long = "brand", value_name = "SLUG")]
    pub brands: Vec<String>,

    /// Fuel products read per brand; every one when omitted.
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Read the labels and print what would be proposed; write nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// What the run did, counted.
#[derive(Debug, Default, PartialEq, Eq)]
struct Tally {
    brands_failed: usize,
    products_listed: usize,
    products_seen: usize,
    candidates: usize,
    proposed: usize,
    skipped_duplicate: usize,
    skipped_illegible: usize,
    skipped_no_photo: usize,
    llm_calls: usize,
    discarded_no_vision: usize,
    /// Reads that errored on every tier or answered with no usable object.
    read_failed: usize,
    /// Candidates that failed outside the read (download, database).
    failed: usize,
}

impl Tally {
    /// The run's verdict: an error when labels were sent and none was read,
    /// or when no brand could be listed.
    fn verdict(&self, brands: usize) -> AppResult<()> {
        let unread = self.discarded_no_vision + self.read_failed;
        if self.llm_calls > 0 && unread >= self.llm_calls {
            return Err(AppError::external_service(
                "llm",
                format!(
                    "every label read failed ({} sent, {} discarded for no vision, {} failed)",
                    self.llm_calls, self.discarded_no_vision, self.read_failed
                ),
            ));
        }
        if brands > 0 && self.brands_failed == brands {
            return Err(AppError::external_service(
                "shopify",
                "no brand storefront could be listed",
            ));
        }
        Ok(())
    }
}

/// The brands `slugs` names, every brand when it names none.
///
/// # Errors
/// Returns an error naming a slug the table does not hold.
fn selected_brands(slugs: &[String]) -> AppResult<Vec<&'static Brand>> {
    if slugs.is_empty() {
        return Ok(BRANDS.iter().collect());
    }
    slugs
        .iter()
        .map(|slug| {
            BRANDS
                .iter()
                .find(|brand| brand.slug.eq_ignore_ascii_case(slug))
                .ok_or_else(|| {
                    let known: Vec<_> = BRANDS.iter().map(|brand| brand.slug).collect();
                    AppError::invalid_input(format!(
                        "unknown brand '{slug}'; known: {}",
                        known.join(", ")
                    ))
                })
        })
        .collect()
}

/// The dedupe hash of a package: its page URL, then each photo sent, in
/// order, each prefixed with its length so two photo sets never collide by
/// concatenation.
fn source_hash(page_url: &str, images: &[LabelImage]) -> String {
    let mut sent = Vec::with_capacity(images.iter().map(|image| image.bytes.len() + 8).sum());
    for image in images {
        sent.extend_from_slice(&(image.bytes.len() as u64).to_le_bytes());
        sent.extend_from_slice(&image.bytes);
    }
    proposal_source_hash(Some(page_url), &sent)
}

/// The text a label printed, or `fallback` when it printed none.
fn printed_or(printed: Option<&str>, fallback: &str) -> String {
    printed
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

/// Everything one run holds.
struct Run {
    repos: Arc<RepositoryRegistry>,
    provider: ChatProvider,
    prompt: String,
    http: reqwest::Client,
    /// `--dry-run`: print what would be proposed, write nothing.
    preview: bool,
    tally: Tally,
}

/// Run `pierre-cli fuel-ingest`.
///
/// # Errors
/// Returns an error for an unknown brand, when the database or the LLM chain
/// cannot be opened, or per [`Tally::verdict`].
pub(crate) async fn run(args: FuelIngestArgs, database_url: &str) -> AppResult<()> {
    let brands = selected_brands(&args.brands)?;
    info!(
        "Connecting to database for fuel ingestion: {}",
        redact_url(database_url)
    );
    let db = Database::init_for_seeding(database_url).await?;
    let mut run = Run {
        repos: Arc::clone(db.repositories()),
        provider: ChatProvider::from_chain(&LlmChain::from_env()).await?,
        prompt: PromptRegistry::new().fuel_panel_extraction_prompt(),
        http: shopify::http_client()?,
        preview: args.dry_run,
        tally: Tally::default(),
    };
    for brand in &brands {
        run.brand(brand, args.limit).await;
    }
    let t = &run.tally;
    info!(
        dry_run = run.preview,
        brands = brands.len(),
        brands_failed = t.brands_failed,
        listed = t.products_listed,
        products_seen = t.products_seen,
        candidates = t.candidates,
        proposed = t.proposed,
        skipped_duplicate = t.skipped_duplicate,
        skipped_illegible = t.skipped_illegible,
        skipped_no_photo = t.skipped_no_photo,
        llm_calls = t.llm_calls,
        discarded_no_vision = t.discarded_no_vision,
        read_failed = t.read_failed,
        failed = t.failed,
        "fuel ingestion finished"
    );
    t.verdict(brands.len())
        .inspect_err(|e| error!("fuel ingestion failed: {e}"))
}

impl Run {
    async fn brand(&mut self, brand: &Brand, limit: Option<usize>) {
        let Some(products) = self.list(brand, limit).await else {
            return;
        };
        for product in &products {
            for candidate in label_candidates(brand.store, product) {
                self.tally.candidates += 1;
                if let Err(e) = self.candidate(brand, &candidate).await {
                    error!(
                        brand = brand.slug,
                        page = %redact_url(&candidate.page_url),
                        "label candidate failed: {e}"
                    );
                    self.tally.failed += 1;
                }
            }
        }
    }

    /// The brand's fuel products, counted; `None` (logged) when the
    /// storefront could not be listed.
    async fn list(&mut self, brand: &Brand, limit: Option<usize>) -> Option<Vec<ShopifyProduct>> {
        match fetch_fuel_products(&self.http, brand, limit).await {
            Ok((listed, products)) => {
                info!(
                    brand = brand.slug,
                    listed,
                    fuel = products.len(),
                    "storefront listed"
                );
                self.tally.products_listed += listed;
                self.tally.products_seen += products.len();
                Some(products)
            }
            Err(e) => {
                error!(brand = brand.slug, "storefront listing failed: {e}");
                self.tally.brands_failed += 1;
                None
            }
        }
    }

    async fn photos(&self, candidate: &LabelCandidate) -> AppResult<Vec<LabelImage>> {
        let mut images = Vec::with_capacity(candidate.image_urls.len());
        for src in &candidate.image_urls {
            if let Some(image) = download_image(&self.http, src).await? {
                images.push(image);
            }
        }
        Ok(images)
    }

    /// The photos to read and their source hash, or `None` (counted) when no
    /// photo was usable or these photos were already proposed.
    async fn fresh_photos(
        &mut self,
        brand: &Brand,
        candidate: &LabelCandidate,
    ) -> AppResult<Option<(Vec<LabelImage>, String)>> {
        let page = redact_url(&candidate.page_url);
        let images = self.photos(candidate).await?;
        if images.is_empty() {
            warn!(brand = brand.slug, %page, "no usable photo, skipped");
            self.tally.skipped_no_photo += 1;
            return Ok(None);
        }
        let hash = source_hash(&candidate.page_url, &images);
        if self
            .repos
            .fuel_catalogue
            .fuel_proposal_exists_by_hash(&hash)
            .await?
        {
            info!(brand = brand.slug, %page, "already proposed from these photos, skipped");
            self.tally.skipped_duplicate += 1;
            return Ok(None);
        }
        Ok(Some((images, hash)))
    }

    /// The legible label `images` show, or `None` (counted, logged) when the
    /// read failed, came from a tier that cannot see images, or found no
    /// legible panel.
    async fn read(
        &mut self,
        brand: &Brand,
        candidate: &LabelCandidate,
        images: &[LabelImage],
    ) -> AppResult<Option<Reading>> {
        let page = redact_url(&candidate.page_url);
        let request = label_request(&self.provider, &self.prompt, images)?;
        self.tally.llm_calls += 1;
        let read = match read_label(&self.provider, &request, &candidate.page_url).await {
            Ok(read) => read,
            Err(ReadFailure::NoVision) => {
                self.tally.discarded_no_vision += 1;
                return Ok(None);
            }
            Err(ReadFailure::Provider(e) | ReadFailure::Unparseable(e)) => {
                error!(brand = brand.slug, %page, "label read failed: {e}");
                self.tally.read_failed += 1;
                return Ok(None);
            }
        };
        if !read.label.legible {
            info!(
                brand = brand.slug,
                %page,
                provider = read.provider,
                "label not legible in these photos, skipped"
            );
            self.tally.skipped_illegible += 1;
            return Ok(None);
        }
        Ok(Some(read))
    }

    async fn candidate(&mut self, brand: &Brand, candidate: &LabelCandidate) -> AppResult<()> {
        let Some((images, hash)) = self.fresh_photos(brand, candidate).await? else {
            return Ok(());
        };
        let Some(read) = self.read(brand, candidate, &images).await? else {
            return Ok(());
        };
        let proposal = proposal(brand, candidate, &read, &images, hash);
        if self.preview {
            println!("{}", preview_line(&proposal, &read, images.len()));
        } else {
            let id = self
                .repos
                .fuel_catalogue
                .create_fuel_proposal(&proposal)
                .await?;
            info!(
                brand = brand.slug,
                page = %redact_url(&candidate.page_url),
                proposal = %id,
                flags = proposal.gate_flags.len(),
                provider = read.provider,
                "fuel proposal filed"
            );
        }
        self.tally.proposed += 1;
        Ok(())
    }
}

/// The proposal a read label files. The stored photo is the last one sent.
fn proposal(
    brand: &Brand,
    candidate: &LabelCandidate,
    read: &Reading,
    images: &[LabelImage],
    hash: String,
) -> NewFuelProposal {
    let label = &read.label;
    let mapped = map_reading(label, &candidate.page_url);
    NewFuelProposal {
        product_id: None,
        brand: brand.display.to_owned(),
        name: printed_or(label.name.as_deref(), &candidate.title),
        variant: printed_or(
            label.variant.as_deref(),
            candidate.flavour.as_deref().unwrap_or_default(),
        ),
        form: label.form,
        gtin: mapped.gtin,
        source_kind: ProposalSource::ShopifyOcr,
        source_url: Some(candidate.page_url.clone()),
        source_image: images.last().map(|image| ProposalSourceImage {
            mime: image.mime.clone(),
            bytes: image.bytes.clone(),
        }),
        source_hash: Some(hash),
        facts: mapped.facts,
        provenance: mapped.provenance,
        gate_flags: mapped.gate_flags,
        proposed_by_user_id: None,
    }
}

/// The `--dry-run` line for a proposal that would be filed.
fn preview_line(proposal: &NewFuelProposal, read: &Reading, photos: usize) -> serde_json::Value {
    json!({
        "would_propose": {
            "brand": proposal.brand,
            "name": proposal.name,
            "variant": proposal.variant,
            "form": proposal.form,
            "gtin": proposal.gtin,
            "source_url": proposal.source_url,
            "photos_sent": photos,
            "stored_photo_bytes": proposal.source_image.as_ref().map(|image| image.bytes.len()),
            "facts": proposal.facts,
            "gate_flags": proposal.gate_flags,
            "label_brand": read.label.brand,
            "multiple_transportable_stated": read.label.multiple_transportable_stated,
            "notes": read.label.notes,
            "served_by": read.provider,
            "served_tier_position": read.position,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(bytes: &[u8]) -> LabelImage {
        LabelImage {
            mime: "image/png".to_owned(),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn every_brand_slug_is_unique_and_selectable() {
        let all = selected_brands(&[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(all.len(), BRANDS.len());
        let picked = selected_brands(&["GU".to_owned(), "naak".to_owned()])
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            picked.iter().map(|b| b.slug).collect::<Vec<_>>(),
            vec!["gu", "naak"]
        );
        assert!(selected_brands(&["maurten".to_owned()]).is_err());
    }

    #[test]
    fn the_source_hash_covers_the_page_and_every_photo_in_order() {
        let url = "https://naak.com/products/gel";
        let a = [image(b"front"), image(b"panel")];
        assert_eq!(source_hash(url, &a), source_hash(url, &a));
        assert_ne!(
            source_hash(url, &a),
            source_hash(url, &[image(b"panel"), image(b"front")])
        );
        assert_ne!(
            source_hash(url, &a),
            source_hash("https://naak.com/products/gel?variant=2", &a)
        );
        // Length prefixes: moving a byte between photos is a different set.
        assert_ne!(
            source_hash(url, &[image(b"ab"), image(b"c")]),
            source_hash(url, &[image(b"a"), image(b"bc")])
        );
    }

    #[test]
    fn the_run_fails_only_when_every_read_failed_or_no_brand_listed() {
        let mut tally = Tally {
            llm_calls: 3,
            read_failed: 2,
            discarded_no_vision: 1,
            ..Tally::default()
        };
        assert!(tally.verdict(1).is_err());
        tally.proposed = 1;
        tally.read_failed = 1;
        assert!(tally.verdict(1).is_ok());
        // A download or database failure is not a failed read.
        assert!(Tally {
            llm_calls: 1,
            proposed: 1,
            failed: 4,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        // Duplicates and illegible labels are not failures; no call is not one.
        assert!(Tally {
            skipped_duplicate: 5,
            ..Tally::default()
        }
        .verdict(1)
        .is_ok());
        assert!(Tally {
            brands_failed: 2,
            ..Tally::default()
        }
        .verdict(2)
        .is_err());
    }

    #[test]
    fn a_printed_identity_wins_over_the_listing() {
        assert_eq!(
            printed_or(Some(" ULTRA Gel "), "ULTRA™ Gel 200 · Maple"),
            "ULTRA Gel"
        );
        assert_eq!(printed_or(Some("  "), "Listing"), "Listing");
        assert_eq!(printed_or(None, ""), "");
    }
}
