// ABOUTME: Shopify storefront discovery for `pierre-cli fuel-ingest` — /products.json pages, the fuel filter, label candidates
// ABOUTME: Splits a product into one candidate per flavour and downloads its label photos, resized, typed and size-capped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The storefront half of the ingestion job.
//!
//! A brand's public `/products.json` lists every product with its options,
//! variants and photos (never a barcode: the storefront endpoint does not
//! serve `variants[].barcode`, verified live on all four checked-in brands on
//! 2026-10-29). [`fetch_products`] pages through it, [`is_fuel`] keeps the
//! fuel and drops the gear, bundles and hidden listings, and
//! [`label_candidates`] turns one product into the packages whose labels are
//! read: one per flavour when the photos are tied to flavours (GU lists 21
//! flavours of one gel as one product), else the product itself, with the
//! flavours its untied photos may mix named for the reviewer.

use std::collections::BTreeSet;
use std::time::Duration;

use pierre_core::constants::project::user_agent;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use tokio::time::sleep;
use tracing::{debug, warn};

use super::Brand;

/// Products per `/products.json` page — Shopify's maximum.
const PAGE_SIZE: u32 = 250;

/// Pages read per brand before the listing is treated as runaway.
const MAX_PAGES: u32 = 20;

/// Attempts per request when the store answers 429 or 503.
const MAX_ATTEMPTS: u32 = 4;

/// The longest `Retry-After` honoured, seconds.
const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Width every photo is requested at through the Shopify CDN: enough for a
/// nutrition panel's small print, well under the providers' image limits.
const IMAGE_WIDTH: &str = "1600";

/// Photos sent per label candidate — the last ones, where the panel usually is.
pub(super) const MAX_IMAGES_PER_CANDIDATE: usize = 4;

/// The largest photo accepted, bytes. Anthropic refuses an image whose
/// base64 encoding is over 5 MiB (5,242,880 bytes), and base64 grows a photo
/// by 4/3: 3.9 MB encodes to 5.2 MB, just under.
const MAX_IMAGE_BYTES: usize = 3_900_000;

/// Media types a photo may have. GIF is refused: a label is never animated,
/// and the readers that take images all take these three.
const ALLOWED_IMAGE_TYPES: [&str; 3] = ["image/png", "image/jpeg", "image/webp"];

/// Words in a listing's type or title that mark it as not one fuel package,
/// whatever the brand: gear, bundles and variety packs (several labels in one
/// photo), samples, recovery and protein products, and capsule supplements
/// (amino acids, vitamins, a daily micronutrient blend) that are not race
/// fuel. Salt and electrolyte capsules stay: GU's "Roctane Electrolyte
/// Capsules" is kept while its "BCAA Capsules" and "Magnesium Plus Capsules"
/// (magnesium, zinc and vitamins D and K, "a simple, daily way to cover your
/// micronutrient bases") are dropped — the only capsule listings in the four
/// checked-in stores, live on 2026-10-29.
///
/// Matched against the type and title only, never the tags: a brand tags its
/// fuel with what is in it, and Näak tags every ULTRA gel, drink mix, waffle,
/// puree and bar "Plant-based Protein" (bars also "Recovery"), so a tag match
/// dropped 25 of its 33 fuel listings (live, 2026-10-29).
const EXCLUDED_WORDS: [&str; 34] = [
    "gear",
    "apparel",
    "accessories",
    "bottle",
    "flask",
    "bundle",
    "bundles",
    "pack",
    "variety",
    "discovery",
    "sample",
    "samples",
    "kit",
    "gift card",
    "cookbook",
    "cookbooks",
    "shirt",
    "tee",
    "hat",
    "socks",
    "stickers",
    "recovery",
    "protein",
    "mixed box",
    "box of",
    "rental",
    "subscription",
    "bcaa",
    "bcaas",
    "amino",
    "aminos",
    "vitamin",
    "vitamins",
    "magnesium plus",
];

/// Tags a storefront puts on a listing it hides from shoppers (internal,
/// partner and retired products). Matched anywhere, tags included.
const HIDDEN_MARKERS: [&str; 3] = ["hide_product", "hide_from_search", "private"];

/// One `/products.json` page.
#[derive(Debug, Deserialize)]
pub(super) struct ProductsPage {
    pub products: Vec<ShopifyProduct>,
}

/// A storefront product, the fields the job reads.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ShopifyProduct {
    pub id: u64,
    pub title: String,
    pub handle: String,
    #[serde(default)]
    pub product_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub options: Vec<ShopifyOption>,
    #[serde(default)]
    pub variants: Vec<ShopifyVariant>,
    #[serde(default)]
    pub images: Vec<ShopifyImage>,
}

/// A product option ("Flavor", "Size") and its values.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ShopifyOption {
    pub name: String,
    pub position: u8,
}

/// A purchasable variant: one combination of option values.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ShopifyVariant {
    pub id: u64,
    pub option1: Option<String>,
    pub option2: Option<String>,
    pub option3: Option<String>,
}

impl ShopifyVariant {
    fn option_value(&self, position: u8) -> Option<&str> {
        match position {
            1 => self.option1.as_deref(),
            2 => self.option2.as_deref(),
            3 => self.option3.as_deref(),
            _ => None,
        }
    }
}

/// A product photo and the variants it shows (empty: the whole product).
#[derive(Debug, Clone, Deserialize)]
pub(super) struct ShopifyImage {
    pub src: String,
    pub position: u32,
    #[serde(default)]
    pub variant_ids: Vec<u64>,
}

/// One package whose label is read: a product, or one flavour of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LabelCandidate {
    /// The product page, pinned to the flavour's first variant when there is one.
    pub page_url: String,
    /// The flavour the photos show, when the product lists flavours.
    pub flavour: Option<String>,
    /// The listing title.
    pub title: String,
    /// The photos to send, in position order, at most [`MAX_IMAGES_PER_CANDIDATE`].
    pub image_urls: Vec<String>,
    /// The flavours the listing offers when its photos are tied to none of
    /// them: the photos may show any of these, so the read is filed with a
    /// note saying so. Empty when the photos show one flavour.
    pub untied_flavours: Vec<String>,
}

impl LabelCandidate {
    /// The ingestion job's note to the reviewer about these photos, if any.
    pub(super) fn photo_note(&self) -> Option<String> {
        (!self.untied_flavours.is_empty()).then(|| {
            format!(
                "The listing offers {} flavours ({}) and its photos are not tied to one, so \
                 they may show different flavours: check the variant against the panel.",
                self.untied_flavours.len(),
                self.untied_flavours.join(", ")
            )
        })
    }
}

/// A downloaded photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LabelImage {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// `text` lowercased with every run of non-alphanumerics (underscores kept,
/// so `hide_product` stays one word) folded to one space, and padded, so a
/// keyword matches whole words only: "hat" never matches "that".
fn word_text(text: &str) -> String {
    let mut out = String::from(" ");
    for c in text.to_lowercase().chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

fn has_word(words: &str, keyword: &str) -> bool {
    words.contains(&word_text(keyword))
}

/// Whether `product` is one fuel package the brand's filter keeps: one of the
/// brand's keywords in its type, tags or title, none of [`EXCLUDED_WORDS`] in
/// its type or title, and none of [`HIDDEN_MARKERS`] anywhere.
pub(super) fn is_fuel(brand: &Brand, product: &ShopifyProduct) -> bool {
    let named = word_text(&format!("{} {}", product.product_type, product.title));
    let words = word_text(&format!(
        "{} {} {}",
        product.product_type,
        product.tags.join(" "),
        product.title
    ));
    if EXCLUDED_WORDS.iter().any(|kw| has_word(&named, kw))
        || HIDDEN_MARKERS.iter().any(|kw| has_word(&words, kw))
    {
        return false;
    }
    brand.keywords.iter().any(|kw| has_word(&words, kw))
}

/// The product page of `handle` on `store`.
pub(super) fn product_url(store: &str, handle: &str) -> String {
    format!("{}/products/{handle}", store.trim_end_matches('/'))
}

/// The option that names flavours, by its position (1-based).
fn flavour_option(product: &ShopifyProduct) -> Option<u8> {
    product
        .options
        .iter()
        .find(|option| {
            let name = option.name.to_lowercase();
            ["flavor", "flavour", "saveur", "taste", "sabor"]
                .iter()
                .any(|kw| name.contains(kw))
        })
        .map(|option| option.position)
}

/// The last [`MAX_IMAGES_PER_CANDIDATE`] of `images`, in position order.
fn last_images<'a>(images: impl Iterator<Item = &'a ShopifyImage>) -> Vec<String> {
    let mut sorted: Vec<&ShopifyImage> = images.collect();
    sorted.sort_by_key(|image| image.position);
    let skip = sorted.len().saturating_sub(MAX_IMAGES_PER_CANDIDATE);
    sorted
        .into_iter()
        .skip(skip)
        .map(|image| image.src.clone())
        .collect()
}

/// The flavours `product`'s variants name at option `position`, in variant
/// order, each once, an assortment ("Mixed Box") left out: its photos show
/// several labels.
fn listed_flavours(product: &ShopifyProduct, position: u8) -> Vec<&str> {
    let mut flavours: Vec<&str> = Vec::new();
    for variant in &product.variants {
        let Some(value) = variant.option_value(position) else {
            continue;
        };
        if flavours.contains(&value) {
            continue;
        }
        let words = word_text(value);
        if EXCLUDED_WORDS.iter().any(|kw| has_word(&words, kw)) {
            debug!(
                product = product.id,
                flavour = value,
                "flavour is an assortment, skipped"
            );
            continue;
        }
        flavours.push(value);
    }
    flavours
}

/// The packages `product` shows labels for.
///
/// When the product lists two or more flavours and its photos are tied to
/// variants, each flavour is its own candidate with the photos of its
/// variants, and a flavour no photo shows is left out (a product-wide photo
/// shows some other flavour). Otherwise the product is one candidate with all
/// its photos: of its one flavour when it lists one, else carrying the
/// flavours its untied photos may mix, so the reviewer is told.
pub(super) fn label_candidates(store: &str, product: &ShopifyProduct) -> Vec<LabelCandidate> {
    let page = product_url(store, &product.handle);
    let tied = product
        .images
        .iter()
        .any(|image| !image.variant_ids.is_empty());
    let flavours = flavour_option(product)
        .map(|position| (position, listed_flavours(product, position)))
        .filter(|(_, flavours)| !flavours.is_empty());
    let split = flavours
        .as_ref()
        .is_some_and(|(_, listed)| tied && listed.len() > 1);
    let Some((position, flavours)) = flavours.clone().filter(|_| split) else {
        let image_urls = last_images(product.images.iter());
        if image_urls.is_empty() {
            return Vec::new();
        }
        let listed = flavours.map(|(_, flavours)| flavours).unwrap_or_default();
        let (flavour, untied_flavours) = match listed.as_slice() {
            [one] => (Some((*one).to_owned()), Vec::new()),
            many => (None, many.iter().map(|f| (*f).to_owned()).collect()),
        };
        return vec![LabelCandidate {
            page_url: page,
            flavour,
            title: product.title.clone(),
            image_urls,
            untied_flavours,
        }];
    };

    let mut candidates = Vec::new();
    for flavour in flavours {
        let variant_ids: BTreeSet<u64> = product
            .variants
            .iter()
            .filter(|variant| variant.option_value(position) == Some(flavour))
            .map(|variant| variant.id)
            .collect();
        let image_urls = last_images(
            product
                .images
                .iter()
                .filter(|image| image.variant_ids.iter().any(|id| variant_ids.contains(id))),
        );
        let Some(first_variant) = variant_ids.first() else {
            continue;
        };
        if image_urls.is_empty() {
            debug!(
                product = product.id,
                flavour, "no photo shows this flavour, skipped"
            );
            continue;
        }
        candidates.push(LabelCandidate {
            page_url: format!("{page}?variant={first_variant}"),
            flavour: Some(flavour.to_owned()),
            title: product.title.clone(),
            image_urls,
            untied_flavours: Vec::new(),
        });
    }
    candidates
}

/// The storefront HTTP client: the platform's user agent, bounded timeouts,
/// rustls (reqwest 0.12, as the rest of pierre-cli).
///
/// # Errors
/// Returns an error when the client cannot be built.
pub(super) fn http_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(user_agent())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::internal(format!("failed to build the storefront client: {e}")))
}

/// How long to wait before retry `attempt` (1-based): the store's
/// `Retry-After` seconds when it sends them, capped, else 2, 4, 8 s.
fn backoff(response: &reqwest::Response, attempt: u32) -> Duration {
    let stated = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok());
    Duration::from_secs(stated.map_or_else(
        || 2_u64.saturating_pow(attempt),
        |secs| secs.min(MAX_RETRY_AFTER_SECS),
    ))
}

/// GET `url`, retrying a 429 or 503 with backoff.
async fn get_with_backoff(client: &reqwest::Client, url: &Url) -> AppResult<reqwest::Response> {
    let mut attempt = 1;
    loop {
        let response = client.get(url.clone()).send().await.map_err(|e| {
            AppError::external_service(
                "shopify",
                format!("GET {} failed: {e}", redact_url(url.as_str())),
            )
        })?;
        let status = response.status();
        let throttled = matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
        );
        if !throttled || attempt >= MAX_ATTEMPTS {
            if status.is_success() {
                return Ok(response);
            }
            return Err(AppError::external_service(
                "shopify",
                format!("GET {} answered {status}", redact_url(url.as_str())),
            ));
        }
        let wait = backoff(&response, attempt);
        warn!(
            url = %redact_url(url.as_str()),
            %status,
            attempt,
            wait_secs = wait.as_secs(),
            "storefront throttled the request, backing off"
        );
        sleep(wait).await;
        attempt += 1;
    }
}

/// The URL of `/products.json` page `page` on `store`.
fn page_url(store: &str, page: u32) -> AppResult<Url> {
    let mut url = Url::parse(&format!("{}/products.json", store.trim_end_matches('/')))
        .map_err(|e| AppError::config(format!("bad store URL {store}: {e}")))?;
    url.query_pairs_mut()
        .append_pair("limit", &PAGE_SIZE.to_string())
        .append_pair("page", &page.to_string());
    Ok(url)
}

/// Every product the brand's storefront lists, page by page until an empty
/// page, or until `enough` fuel products were found.
///
/// # Errors
/// Returns an error when a page cannot be fetched or parsed.
pub(super) async fn fetch_fuel_products(
    client: &reqwest::Client,
    brand: &Brand,
    enough: Option<usize>,
) -> AppResult<(usize, Vec<ShopifyProduct>)> {
    let mut listed = 0;
    let mut fuel = Vec::new();
    for page in 1..=MAX_PAGES {
        let url = page_url(brand.store, page)?;
        let body = get_with_backoff(client, &url)
            .await?
            .text()
            .await
            .map_err(|e| {
                AppError::external_service(
                    "shopify",
                    format!("reading {} failed: {e}", redact_url(url.as_str())),
                )
            })?;
        let products = parse_products_page(&body)?;
        if products.is_empty() {
            return Ok((listed, fuel));
        }
        listed += products.len();
        for product in products {
            if is_fuel(brand, &product) {
                fuel.push(product);
            } else {
                debug!(brand = brand.slug, title = %product.title, "not a fuel package, skipped");
            }
            if enough.is_some_and(|limit| fuel.len() >= limit) {
                return Ok((listed, fuel));
            }
        }
    }
    warn!(
        brand = brand.slug,
        pages = MAX_PAGES,
        "storefront listing did not end within the page cap; continuing with what was read"
    );
    Ok((listed, fuel))
}

/// The products of one `/products.json` body.
///
/// # Errors
/// Returns an error when the body is not a products page.
pub(super) fn parse_products_page(body: &str) -> AppResult<Vec<ShopifyProduct>> {
    serde_json::from_str::<ProductsPage>(body)
        .map(|page| page.products)
        .map_err(|e| AppError::external_service("shopify", format!("not a products page: {e}")))
}

/// `src` asked for at [`IMAGE_WIDTH`] through the Shopify CDN's `width`
/// parameter, beside the cache-busting `v` it already carries.
///
/// # Errors
/// Returns an error when `src` is not a URL.
pub(super) fn sized_image_url(src: &str) -> AppResult<Url> {
    let absolute = if src.starts_with("//") {
        format!("https:{src}")
    } else {
        src.to_owned()
    };
    let mut url = Url::parse(&absolute)
        .map_err(|e| AppError::external_service("shopify", format!("bad image URL: {e}")))?;
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "width")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(kept)
        .append_pair("width", IMAGE_WIDTH);
    Ok(url)
}

/// The media type a response declares, without parameters, when it is one a
/// label photo may have.
pub(super) fn accepted_mime(content_type: Option<&str>) -> Option<String> {
    let mime = content_type?.split(';').next()?.trim().to_ascii_lowercase();
    ALLOWED_IMAGE_TYPES.contains(&mime.as_str()).then_some(mime)
}

/// Download one photo, sized; `None` (logged) when its type is not accepted
/// or it is over [`MAX_IMAGE_BYTES`].
///
/// # Errors
/// Returns an error when the photo cannot be fetched.
pub(super) async fn download_image(
    client: &reqwest::Client,
    src: &str,
) -> AppResult<Option<LabelImage>> {
    let url = sized_image_url(src)?;
    let response = get_with_backoff(client, &url).await?;
    let declared = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let Some(mime) = accepted_mime(declared.as_deref()) else {
        warn!(
            url = %redact_url(url.as_str()),
            content_type = declared.as_deref().unwrap_or("none"),
            "photo is not png, jpeg or webp, skipped"
        );
        return Ok(None);
    };
    if response
        .content_length()
        .is_some_and(|len| len > MAX_IMAGE_BYTES as u64)
    {
        warn!(url = %redact_url(url.as_str()), "photo is over the size cap, skipped");
        return Ok(None);
    }
    let bytes = response.bytes().await.map_err(|e| {
        AppError::external_service(
            "shopify",
            format!("reading {} failed: {e}", redact_url(url.as_str())),
        )
    })?;
    if bytes.len() > MAX_IMAGE_BYTES {
        warn!(url = %redact_url(url.as_str()), bytes = bytes.len(), "photo is over the size cap, skipped");
        return Ok(None);
    }
    Ok(Some(LabelImage {
        mime,
        bytes: bytes.to_vec(),
    }))
}

#[cfg(test)]
mod tests {
    use super::super::BRANDS;
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/products_page.json");

    fn brand(slug: &str) -> &'static Brand {
        BRANDS
            .iter()
            .find(|brand| brand.slug == slug)
            .unwrap_or_else(|| panic!("no brand {slug}"))
    }

    fn fixture() -> Vec<ShopifyProduct> {
        parse_products_page(FIXTURE).unwrap_or_else(|e| panic!("fixture parses: {e}"))
    }

    fn product(title: &str) -> ShopifyProduct {
        fixture()
            .into_iter()
            .find(|product| product.title == title)
            .unwrap_or_else(|| panic!("no product {title}"))
    }

    #[test]
    fn a_products_page_parses_without_a_barcode() {
        let products = fixture();
        assert_eq!(products.len(), 12);
        let gel = product("ULTRA™ Gel 200 · Salted Maple");
        assert_eq!(gel.handle, "ultra-gel-200-salted-maple");
        assert_eq!(gel.images.len(), 2);
        assert_eq!(gel.product_type, "Gels");
    }

    #[test]
    fn an_empty_page_ends_the_listing_and_junk_is_refused() {
        assert!(parse_products_page(r#"{"products":[]}"#)
            .unwrap_or_else(|e| panic!("{e}"))
            .is_empty());
        assert!(parse_products_page("<html>not a store</html>").is_err());
    }

    #[test]
    fn the_filter_keeps_fuel_and_drops_gear_bundles_and_hidden_listings() {
        let naak = brand("naak");
        // Its live tags include "Plant-based Protein": a tag says what is in
        // the fuel, never that it is not fuel.
        let gel = product("ULTRA™ Gel 200 · Salted Maple");
        assert!(gel.tags.iter().any(|tag| tag == "Plant-based Protein"));
        assert!(is_fuel(naak, &gel));
        let mut recovery_tagged = gel;
        recovery_tagged.tags.push("Recovery".to_owned());
        assert!(is_fuel(naak, &recovery_tagged));
        assert!(!is_fuel(
            naak,
            &product("ULTRA Soft Flask · HydraPak® · 500ml")
        ));
        assert!(!is_fuel(naak, &product("Boost™ ‧ Marathon Pack")));
        let tailwind = brand("tailwind");
        assert!(!is_fuel(
            tailwind,
            &product("Endurance Fuel: Caffeinated Blood Orange")
        ));
        let gu = brand("gu");
        assert!(is_fuel(gu, &product("Original Energy Gel")));
        assert!(!is_fuel(gu, &product("GU Variety Energy Box")));
        // A title that names recovery or protein is not race fuel, whatever
        // keyword ("drink mix") it also carries.
        assert!(!is_fuel(gu, &product("Roctane Protein Recovery Drink Mix")));
        // Salt capsules are fuel; amino-acid and micronutrient capsules are not.
        assert!(is_fuel(gu, &product("Roctane Electrolyte Capsules")));
        assert!(!is_fuel(gu, &product("Roctane BCAA Capsules")));
        assert!(!is_fuel(gu, &product("Roctane Magnesium Plus Capsules")));
    }

    #[test]
    fn keywords_match_whole_words_only() {
        let words = word_text("Salted Maple · that chat");
        assert!(!has_word(&words, "hat"));
        assert!(has_word(&words, "salted maple"));
        assert!(has_word(&word_text("hide_product,private"), "hide_product"));
    }

    #[test]
    fn a_product_without_flavour_photos_is_one_candidate_of_its_last_photos() {
        let candidates = label_candidates(
            "https://naak.com/",
            &product("ULTRA™ Gel 200 · Salted Maple"),
        );
        assert_eq!(candidates.len(), 1);
        let only = &candidates[0];
        assert_eq!(
            only.page_url,
            "https://naak.com/products/ultra-gel-200-salted-maple"
        );
        assert_eq!(only.flavour, None);
        assert_eq!(only.untied_flavours, Vec::<String>::new());
        assert_eq!(only.photo_note(), None);
        assert_eq!(only.image_urls.len(), 2);
        assert!(
            only.image_urls[1].contains("panel"),
            "{:?}",
            only.image_urls
        );
    }

    #[test]
    fn flavour_photos_split_a_product_into_one_candidate_per_flavour() {
        let candidates = label_candidates("https://guenergy.com", &product("Original Energy Gel"));
        let flavours: Vec<_> = candidates
            .iter()
            .map(|c| c.flavour.clone().unwrap_or_default())
            .collect();
        // Tri-Berry has photos; Cola Me Happy has none tied to it; the mixed
        // box is an assortment.
        assert_eq!(
            flavours,
            vec!["Tri-Berry".to_owned(), "Salted Caramel".to_owned()]
        );
        let tri_berry = &candidates[0];
        assert_eq!(
            tri_berry.page_url,
            "https://guenergy.com/products/original-energy-gel?variant=101"
        );
        // Five photos tied to Tri-Berry: the last four are sent, in order.
        assert_eq!(tri_berry.image_urls.len(), MAX_IMAGES_PER_CANDIDATE);
        assert!(tri_berry.image_urls[0].ends_with("tb-2.png?v=1"));
        assert!(tri_berry.image_urls[3].ends_with("tb-panel.png?v=1"));
    }

    #[test]
    fn untied_photos_of_several_flavours_are_one_candidate_that_says_so() {
        let candidates =
            label_candidates("https://tailwindnutrition.com", &product("Endurance Fuel"));
        assert_eq!(candidates.len(), 1);
        let only = &candidates[0];
        assert_eq!(only.flavour, None);
        assert_eq!(only.image_urls.len(), 2);
        assert_eq!(only.untied_flavours, vec!["Berry", "Lemon", "Naked"]);
        let note = only.photo_note().unwrap_or_default();
        assert!(note.contains("3 flavours (Berry, Lemon, Naked)"), "{note}");
    }

    #[test]
    fn a_single_flavour_keeps_every_photo_tied_or_not() {
        // One photo is tied to a variant, one is not: with one flavour listed
        // both show it, so neither is dropped and no note is needed.
        let candidates = label_candidates(
            "https://www.skratchlabs.com",
            &product("Electrolyte Booster"),
        );
        assert_eq!(candidates.len(), 1);
        let only = &candidates[0];
        assert_eq!(only.flavour.as_deref(), Some("Unflavored"));
        assert_eq!(only.image_urls.len(), 2);
        assert_eq!(only.photo_note(), None);
    }

    #[test]
    fn a_product_with_no_photo_has_no_candidate() {
        let mut bare = product("ULTRA™ Gel 200 · Salted Maple");
        bare.images.clear();
        assert_eq!(label_candidates("https://naak.com", &bare), Vec::new());
    }

    #[test]
    fn photos_are_asked_for_at_the_label_width_keeping_their_version() {
        let url = sized_image_url("//cdn.shopify.com/s/files/gel.png?v=17&width=300")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            url.as_str(),
            "https://cdn.shopify.com/s/files/gel.png?v=17&width=1600"
        );
    }

    #[test]
    fn only_png_jpeg_and_webp_are_accepted() {
        assert_eq!(
            accepted_mime(Some("image/png")).as_deref(),
            Some("image/png")
        );
        assert_eq!(
            accepted_mime(Some("Image/JPEG; charset=binary")).as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(
            accepted_mime(Some("image/webp")).as_deref(),
            Some("image/webp")
        );
        assert_eq!(accepted_mime(Some("image/gif")), None);
        assert_eq!(accepted_mime(Some("text/html")), None);
        assert_eq!(accepted_mime(None), None);
    }

    #[test]
    fn a_page_url_carries_the_page_size_and_number() {
        let url = page_url("https://www.skratchlabs.com/", 3).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            url.as_str(),
            "https://www.skratchlabs.com/products.json?limit=250&page=3"
        );
    }
}
