// ABOUTME: The nutrition panel for `pierre-cli fuel-ingest` — each brand's product page shows it as an image products.json never lists
// ABOUTME: Fetches the page once per listing, spaced per store, finds the panel with the brand's locator, and sends it first (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where a label read's panel comes from.
//!
//! The storefront's `/products.json` photos are almost all front-of-pack:
//! not one of the four checked-in brands lists its nutrition panel there, nor
//! in `/products/<handle>.js` or `.json` (live, 2026-10-29). Each shows it on
//! the product page instead, as an image a [`PanelLocator`] finds. The page
//! is fetched once per listing ([`ProductPage`]), at least [`PAGE_SPACING`]
//! after the store's previous page, through the storefront client's
//! `Retry-After`-aware backoff. The read then sends the panel first and at
//! most [`STORE_PHOTOS_WITH_PANEL`] store photos after it, the fronts that
//! confirm brand and flavour ([`gather`]). With no panel — the page shows
//! none, could not be fetched, or its image could not be downloaded, or the
//! listing mixes flavours a per-flavour panel cannot speak for — the
//! read sends the store photos as it always did, and the job's note says why.

use std::collections::HashMap;
use std::time::Duration;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::redaction::redact_url;
use reqwest::Url;
use scraper::{ElementRef, Html, Selector};
use tokio::time::{sleep_until, Instant};
use tracing::{info, warn};

use super::shopify::{
    download_image, download_url, get_with_backoff, sized, LabelCandidate, LabelImage, IMAGE_WIDTH,
    MAX_IMAGE_BYTES, MAX_READ_BYTES,
};
use super::tally::Tally;
use super::Brand;

/// The least time between two product-page requests to one store. A browser
/// user agent at one request a second drew Cloudflare 429s on all four
/// stores; two seconds apart drew none (live, 2026-10-29).
pub(super) const PAGE_SPACING: Duration = Duration::from_secs(2);

/// Store photos sent after a panel: the first ones, the fronts, which let the
/// reader confirm brand and flavour. The panel carries the facts.
pub(super) const STORE_PHOTOS_WITH_PANEL: usize = 2;

// A panel and its store photos always fit the read's byte budget, each photo
// being at most `MAX_IMAGE_BYTES`, so none is left out for size.
const _: () = assert!((1 + STORE_PHOTOS_WITH_PANEL) * MAX_IMAGE_BYTES <= MAX_READ_BYTES);

/// The largest product page read, bytes. The four stores' pages run 0.5 to
/// 1.2 MB (live, 2026-10-29).
const MAX_PAGE_BYTES: usize = 8_000_000;

/// Width an SVG panel is rasterised at by the Shopify CDN (`format=png`):
/// Skratch's 1000 px render is sharp (998 x 1234, live, 2026-10-29).
const SVG_PANEL_WIDTH: &str = "1000";

/// Where a brand's product page shows its nutrition panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PanelLocator {
    /// Näak: `img.product-drawer-composition__nutrition-img` in the
    /// "Ingredients and nutrition facts" drawer, one per page (a Bag and a
    /// Single Serve are separate listings).
    NaakDrawer,
    /// Tailwind: the image of the tab whose label is "Nutrition Facts",
    /// lazy-loaded through `data-src`. The tab ids are UUIDs, so the tab is
    /// found by its label. One panel serves a whole product family.
    TailwindTab,
    /// Skratch: per variant, the image in the "nutrition info" accordion of
    /// `[data-product-accordions="<variant id>"]`, an SVG the CDN rasterises.
    SkratchAccordion,
    /// GU: per variant, `data-variant-info-link` of
    /// `li[data-variant-id="<variant id>"]`.
    GuVariantLink,
}

impl PanelLocator {
    /// Whether one panel serves every flavour of a product, so a product is
    /// read once rather than per flavour, and the panel cannot show what
    /// tells its flavours apart (Tailwind's caffeinated flavours share the
    /// uncaffeinated panel).
    pub(crate) const fn per_family(self) -> bool {
        matches!(self, Self::TailwindTab)
    }

    /// Whether the panel this locator finds can be sent for `candidate`. A
    /// candidate that offers several flavours its photos are tied to none of
    /// has no one flavour, and only a family panel speaks for all of them:
    /// any other locator would find one flavour's panel (the lowest variant
    /// id's, or the page's only one) and file it for the whole listing.
    pub(super) fn serves(self, candidate: &LabelCandidate) -> bool {
        self.per_family() || candidate.untied_flavours.is_empty()
    }
}

/// A CSS selector the locators use.
fn selector(css: &str) -> AppResult<Selector> {
    Selector::parse(css)
        .map_err(|e| AppError::internal(format!("panel selector {css} does not parse: {e}")))
}

/// `name` of `element`, when present and not blank.
fn attr<'a>(element: &ElementRef<'a>, name: &str) -> Option<&'a str> {
    element
        .value()
        .attr(name)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The first image under `scope` with a source: `data-src` (lazy-loaded)
/// before `src`.
fn first_image<'a>(scope: ElementRef<'a>, img: &Selector) -> Option<&'a str> {
    scope
        .select(img)
        .find_map(|image| attr(&image, "data-src").or_else(|| attr(&image, "src")))
}

/// The panel source `locator` finds in `html`: for a per-variant locator,
/// that of the first of `variant_ids` the page has one for.
///
/// # Errors
/// Returns an error when a locator's selector does not parse.
fn find_source(
    locator: PanelLocator,
    html: &Html,
    variant_ids: &[u64],
) -> AppResult<Option<String>> {
    let img = selector("img")?;
    let found = match locator {
        PanelLocator::NaakDrawer => html
            .select(&selector("img.product-drawer-composition__nutrition-img")?)
            .find_map(|image| attr(&image, "src").or_else(|| attr(&image, "data-src"))),
        PanelLocator::TailwindTab => {
            let label = html.select(&selector("a.tab-a")?).find(|tab| {
                tab.text()
                    .collect::<String>()
                    .trim()
                    .eq_ignore_ascii_case("nutrition facts")
            });
            let tabs = selector("div.tab")?;
            label.and_then(|tab| attr(&tab, "data-id")).and_then(|id| {
                html.select(&tabs)
                    .filter(|tab| attr(tab, "data-id") == Some(id))
                    .find_map(|tab| first_image(tab, &img))
            })
        }
        PanelLocator::SkratchAccordion => {
            let blocks = selector("[data-product-accordions]")?;
            let tabs = selector("div.tab")?;
            let nutrition = selector(
                "[aria-controls=\"accordion_content_nutrition-info\"], #accordion_content_nutrition-info",
            )?;
            variant_ids.iter().find_map(|variant| {
                let variant = variant.to_string();
                html.select(&blocks)
                    .filter(|block| attr(block, "data-product-accordions") == Some(&variant))
                    .flat_map(|block| block.select(&tabs))
                    .filter(|tab| tab.select(&nutrition).next().is_some())
                    .find_map(|tab| first_image(tab, &img))
            })
        }
        PanelLocator::GuVariantLink => {
            let items = selector("li[data-variant-id]")?;
            variant_ids.iter().find_map(|variant| {
                let variant = variant.to_string();
                html.select(&items)
                    .filter(|item| attr(item, "data-variant-id") == Some(&variant))
                    .find_map(|item| attr(&item, "data-variant-info-link"))
            })
        }
    };
    Ok(found.map(str::to_owned))
}

/// `src` as the download it is asked for: resolved against `page` (a
/// protocol-relative `//host/…` or a `/path` included), Shopify's legacy
/// size suffix (`Panel_750x.png`) dropped so the CDN starts from the
/// original, and sized: an SVG rasterised to PNG at [`SVG_PANEL_WIDTH`],
/// anything else at [`IMAGE_WIDTH`].
///
/// # Errors
/// Returns an error when `src` does not resolve to a URL.
pub(super) fn panel_url(page: &Url, src: &str) -> AppResult<Url> {
    let mut url = page
        .join(src)
        .map_err(|e| AppError::external_service("shopify", format!("bad panel URL: {e}")))?;
    if let Some(file) = url.path_segments().and_then(Iterator::last) {
        if let Some(original) = without_size_suffix(file) {
            let path = url.path();
            let dir = &path[..path.len() - file.len()];
            url.set_path(&format!("{dir}{original}"));
        }
    }
    let svg = url.path().to_ascii_lowercase().ends_with(".svg");
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "format")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut().clear().extend_pairs(kept);
    if svg {
        url.query_pairs_mut().append_pair("format", "png");
        return Ok(sized(url, SVG_PANEL_WIDTH));
    }
    Ok(sized(url, IMAGE_WIDTH))
}

/// `file` without a Shopify size suffix before its extension
/// (`Nutrition_Facts_750x.png` → `Nutrition_Facts.png`), or `None` when it
/// carries none.
fn without_size_suffix(file: &str) -> Option<String> {
    let (stem, extension) = file.rsplit_once('.')?;
    let (base, size) = stem.rsplit_once('_')?;
    let digits = size.strip_suffix('x')?;
    (!base.is_empty() && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("{base}.{extension}"))
}

/// The panel `locator` finds on the page `html` serves at `page`, as the URL
/// to download; `None` when the page shows none.
///
/// # Errors
/// Returns an error when a selector does not parse or the panel's source is
/// not a URL.
pub(super) fn locate(
    locator: PanelLocator,
    page: &Url,
    html: &str,
    variant_ids: &[u64],
) -> AppResult<Option<Url>> {
    let document = Html::parse_document(html);
    find_source(locator, &document, variant_ids)?
        .map(|src| panel_url(page, &src))
        .transpose()
}

/// Product-page requests, spaced [`PAGE_SPACING`] apart per store.
#[derive(Debug, Default)]
pub(super) struct PagePacer {
    last: HashMap<&'static str, Instant>,
}

impl PagePacer {
    /// Wait until `store` may be asked for another page.
    async fn wait(&self, store: &str) {
        if let Some(last) = self.last.get(store) {
            sleep_until(*last + PAGE_SPACING).await;
        }
    }
}

/// What a listing's product page came to, fetched on first need.
#[derive(Debug)]
enum PageState {
    Unfetched,
    Fetched(String),
    Failed,
}

/// One listing's product page, shared by its flavours' reads.
#[derive(Debug)]
pub(super) struct ProductPage {
    url: String,
    state: PageState,
}

impl ProductPage {
    /// The page at `url`, not fetched yet.
    pub(super) const fn new(url: String) -> Self {
        Self {
            url,
            state: PageState::Unfetched,
        }
    }

    /// The page's HTML, fetched once; `None` (logged) when it could not be.
    async fn html(
        &mut self,
        http: &reqwest::Client,
        pacer: &mut PagePacer,
        store: &'static str,
    ) -> Option<&str> {
        if matches!(self.state, PageState::Unfetched) {
            pacer.wait(store).await;
            let fetched = fetch_page(http, &self.url).await;
            pacer.last.insert(store, Instant::now());
            self.state = match fetched {
                Ok(html) => PageState::Fetched(html),
                Err(e) => {
                    warn!(page = %redact_url(&self.url), "product page fetch failed: {e}");
                    PageState::Failed
                }
            };
        }
        match &self.state {
            PageState::Fetched(html) => Some(html),
            PageState::Unfetched | PageState::Failed => None,
        }
    }
}

/// GET a product page, up to [`MAX_PAGE_BYTES`]: a longer one is refused as
/// it streams in, never buffered whole first.
async fn fetch_page(http: &reqwest::Client, page: &str) -> AppResult<String> {
    let url = Url::parse(page)
        .map_err(|e| AppError::external_service("shopify", format!("bad page URL: {e}")))?;
    let over_cap = || {
        AppError::external_service(
            "shopify",
            format!("{} is over the page size cap", redact_url(page)),
        )
    };
    let mut response = get_with_backoff(http, &url).await?;
    if response
        .content_length()
        .is_some_and(|len| len > MAX_PAGE_BYTES as u64)
    {
        return Err(over_cap());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        AppError::external_service(
            "shopify",
            format!("reading {} failed: {e}", redact_url(page)),
        )
    })? {
        if body.len() + chunk.len() > MAX_PAGE_BYTES {
            return Err(over_cap());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// What a read sends, and the job's note to the reviewer on where its panel
/// came from.
#[derive(Debug)]
pub(super) struct Gathered {
    pub images: Vec<LabelImage>,
    pub note: String,
}

/// Whatever a run holds to fetch pages and photos.
pub(super) struct Fetcher<'a> {
    pub http: &'a reqwest::Client,
    pub pacer: &'a mut PagePacer,
    pub tally: &'a mut Tally,
}

/// The photos a read of `candidate` sends: the panel its product page shows
/// first, then [`STORE_PHOTOS_WITH_PANEL`] store photos; with no panel,
/// every store photo up to [`MAX_READ_BYTES`] ([`store_photos`]).
///
/// # Errors
/// Returns an error when a store photo cannot be fetched.
pub(super) async fn gather(
    mut fetch: Fetcher<'_>,
    brand: &Brand,
    page: &mut ProductPage,
    candidate: &LabelCandidate,
) -> AppResult<Gathered> {
    let (panel, panel_src) = match find_panel(&mut fetch, brand, page, candidate).await? {
        Ok(found) => found,
        Err(why) => {
            return Ok(Gathered {
                images: store_photos(fetch.http, candidate).await?,
                note: format!("{why}; only the store photos were read."),
            });
        }
    };
    fetch.tally.panel_sent += 1;
    let mut images = vec![panel];
    for src in candidate.image_urls.iter().take(STORE_PHOTOS_WITH_PANEL) {
        if let Some(image) = download_image(fetch.http, src).await? {
            images.push(image);
        }
    }
    let mut note = format!(
        "The first photo is the nutrition panel from the product page ({panel_src}); the \
         others are store photos."
    );
    if brand.panel.per_family() {
        note.push_str(
            " This brand's product page shows one panel per product whatever flavour is \
             pictured: it may be headed with another flavour and never shows a caffeinated \
             flavour's caffeine, so check flavour and caffeine against the listing.",
        );
    }
    Ok(Gathered { images, note })
}

/// The panel image `candidate`'s product page shows and where it came from;
/// else (counted, logged) why none is sent, as the reviewer's note opens.
///
/// # Errors
/// Returns an error when a selector does not parse or a URL is malformed.
async fn find_panel(
    fetch: &mut Fetcher<'_>,
    brand: &Brand,
    page: &mut ProductPage,
    candidate: &LabelCandidate,
) -> AppResult<Result<(LabelImage, Url), String>> {
    let shown = redact_url(&candidate.page_url);
    if !brand.panel.serves(candidate) {
        info!(brand = brand.slug, page = %shown, "listing mixes flavours its page has one panel each for; no panel sent");
        fetch.tally.panel_missing += 1;
        return Ok(Err(
            "The product page shows one flavour's nutrition panel and this \
                       listing offers several, so no panel was sent"
                .to_owned(),
        ));
    }
    let base = Url::parse(&page.url)
        .map_err(|e| AppError::external_service("shopify", format!("bad page URL: {e}")))?;
    let Some(html) = page.html(fetch.http, fetch.pacer, brand.store).await else {
        fetch.tally.page_failed += 1;
        return Ok(Err(
            "The product page could not be fetched, so its nutrition panel was not read".to_owned(),
        ));
    };
    let Some(panel_src) = locate(brand.panel, &base, html, &candidate.variant_ids)? else {
        info!(brand = brand.slug, page = %shown, "product page shows no nutrition panel");
        fetch.tally.panel_missing += 1;
        return Ok(Err("The product page shows no nutrition panel".to_owned()));
    };
    match download_url(fetch.http, panel_src.clone()).await {
        Ok(Some(panel)) => Ok(Ok((panel, panel_src))),
        // A refused type or size was logged by the download.
        refused => {
            let why = refused.err().map_or_else(
                || "not png, jpeg or webp, or over the size cap".to_owned(),
                |e| e.to_string(),
            );
            warn!(brand = brand.slug, panel = %redact_url(panel_src.as_str()), "nutrition panel image not usable ({why}); reading the store photos");
            fetch.tally.panel_missing += 1;
            Ok(Err(format!(
                "The product page's nutrition panel ({panel_src}) could not be downloaded as \
                 an image"
            )))
        }
    }
}

/// The candidate's store photos, every one up to [`MAX_READ_BYTES`]
/// together: read from the last, where a panel photo usually is, and kept
/// in the store's order.
async fn store_photos(
    http: &reqwest::Client,
    candidate: &LabelCandidate,
) -> AppResult<Vec<LabelImage>> {
    let mut images = Vec::with_capacity(candidate.image_urls.len());
    let mut bytes = 0;
    for src in candidate.image_urls.iter().rev() {
        let Some(image) = download_image(http, src).await? else {
            continue;
        };
        if bytes + image.bytes.len() > MAX_READ_BYTES {
            warn!(
                page = %redact_url(&candidate.page_url),
                bytes = image.bytes.len(),
                "photo left out: the read's photos are over the byte budget"
            );
            continue;
        }
        bytes += image.bytes.len();
        images.push(image);
    }
    images.reverse();
    Ok(images)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAAK: &str = include_str!("fixtures/panel_naak.html");
    const TAILWIND: &str = include_str!("fixtures/panel_tailwind.html");
    const SKRATCH: &str = include_str!("fixtures/panel_skratch.html");
    const GU: &str = include_str!("fixtures/panel_gu.html");

    fn page(url: &str) -> Url {
        Url::parse(url).unwrap_or_else(|e| panic!("{e}"))
    }

    fn found(locator: PanelLocator, base: &str, html: &str, variants: &[u64]) -> Option<String> {
        locate(locator, &page(base), html, variants)
            .unwrap_or_else(|e| panic!("{e}"))
            .map(String::from)
    }

    #[test]
    fn every_locator_selector_parses() {
        let empty = Html::parse_document("<html></html>");
        for locator in [
            PanelLocator::NaakDrawer,
            PanelLocator::TailwindTab,
            PanelLocator::SkratchAccordion,
            PanelLocator::GuVariantLink,
        ] {
            assert_eq!(
                find_source(locator, &empty, &[1]).unwrap_or_else(|e| panic!("{e}")),
                None,
                "{locator:?}"
            );
        }
    }

    #[test]
    fn naak_reads_the_drawer_image_resolved_and_sized() {
        assert_eq!(
            found(
                PanelLocator::NaakDrawer,
                "https://naak.com/products/ultra-gel-200-salted-maple",
                NAAK,
                &[]
            )
            .as_deref(),
            Some("https://www.naak.com/cdn/shop/files/EGSM-SS-CA-TVN.png?v=1738511171&width=1600")
        );
    }

    #[test]
    fn tailwind_reads_the_nutrition_facts_tab_at_its_original_size() {
        // The tab is found by its label; its lazy image's `_750x` size
        // suffix is dropped so the CDN starts from the 2922 px original.
        assert_eq!(
            found(
                PanelLocator::TailwindTab,
                "https://tailwindnutrition.com/products/high-carb-fuel-12-pack",
                TAILWIND,
                &[]
            )
            .as_deref(),
            Some("https://tailwindnutrition.com/cdn/shop/files/Nutrition_Facts_HCF-Simplified.png?v=1755627394&width=1600")
        );
        // A page whose tabs name no "Nutrition Facts" has no panel.
        let renamed = TAILWIND.replace(">Nutrition Facts<", ">Reviews<");
        assert_eq!(
            found(
                PanelLocator::TailwindTab,
                "https://tailwindnutrition.com/products/x",
                &renamed,
                &[]
            ),
            None
        );
    }

    #[test]
    fn skratch_reads_the_variants_accordion_svg_as_a_png() {
        let base =
            "https://www.skratchlabs.com/products/energy-bar-sport-fuel?variant=41588270301255";
        assert_eq!(
            found(PanelLocator::SkratchAccordion, base, SKRATCH, &[41_588_270_301_255]).as_deref(),
            Some("https://www.skratchlabs.com/cdn/shop/files/EBF-CO-50g-12_110724_NFP.svg?v=1754678193&format=png&width=1000")
        );
        // The first variant with a panel wins; one with no nutrition-info
        // accordion is passed over, and a page with none of them has none.
        assert_eq!(
            found(
                PanelLocator::SkratchAccordion,
                base,
                SKRATCH,
                &[39_951_831_072_839, 41_588_270_268_487]
            )
            .as_deref(),
            Some("https://www.skratchlabs.com/cdn/shop/files/EBF-CO-50g-12_110724_NFP_Single.svg?v=1754678193&format=png&width=1000")
        );
        assert_eq!(
            found(
                PanelLocator::SkratchAccordion,
                base,
                SKRATCH,
                &[39_951_831_072_839]
            ),
            None
        );
    }

    #[test]
    fn gu_reads_the_variants_info_link() {
        let base = "https://guenergy.com/products/energy-gel";
        assert_eq!(
            found(PanelLocator::GuVariantLink, base, GU, &[31_693_107_593_267]).as_deref(),
            Some("https://guenergy.com/cdn/shop/files/Energy_Gels_-_SFP_-_Cola-01.png?v=1743549115&width=1600")
        );
        // A Mixed Box variant's link is empty: the next variant's is used.
        assert_eq!(
            found(
                PanelLocator::GuVariantLink,
                base,
                GU,
                &[31_169_147_830_323, 31_169_146_912_819]
            )
            .as_deref(),
            Some("https://guenergy.com/cdn/shop/files/Energy_Gels_-_SFP_-_ChocolateOutrage-01.png?v=1743549116&width=1600")
        );
        assert_eq!(
            found(PanelLocator::GuVariantLink, base, GU, &[31_169_147_830_323]),
            None
        );
    }

    #[test]
    fn a_locator_finds_nothing_on_another_brands_page() {
        let base = "https://naak.com/products/x";
        assert_eq!(found(PanelLocator::NaakDrawer, base, GU, &[]), None);
        assert_eq!(found(PanelLocator::TailwindTab, base, NAAK, &[]), None);
        assert_eq!(
            found(
                PanelLocator::GuVariantLink,
                base,
                SKRATCH,
                &[41_588_270_268_487]
            ),
            None
        );
    }

    #[test]
    fn a_panel_source_resolves_against_its_page() {
        let base = page("https://www.skratchlabs.com/products/hydration?variant=1");
        let relative = panel_url(&base, "/cdn/shop/files/SDM-AC_NFP.png?v=3&width=500")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            relative.as_str(),
            "https://www.skratchlabs.com/cdn/shop/files/SDM-AC_NFP.png?v=3&width=1600"
        );
        assert_eq!(
            without_size_suffix("Panel_750x.jpg").as_deref(),
            Some("Panel.jpg")
        );
        assert_eq!(without_size_suffix("Gel_12_x.jpg"), None);
        assert_eq!(without_size_suffix("protein_15.png"), None);
        assert_eq!(without_size_suffix("_750x.png"), None);
    }

    #[test]
    fn a_listing_of_mixed_flavours_gets_only_a_family_panel() {
        let mut candidate = LabelCandidate {
            page_url: "https://guenergy.com/products/energy-gel".to_owned(),
            flavour: Some("Cola".to_owned()),
            title: "Energy Gel".to_owned(),
            image_urls: vec!["a".to_owned()],
            variant_ids: vec![31_169_146_912_819, 31_693_107_593_267],
            untied_flavours: Vec::new(),
        };
        for locator in [
            PanelLocator::NaakDrawer,
            PanelLocator::TailwindTab,
            PanelLocator::SkratchAccordion,
            PanelLocator::GuVariantLink,
        ] {
            assert!(locator.serves(&candidate), "{locator:?}");
        }
        // Untied photos of two flavours: the lowest variant's panel (here
        // Chocolate Outrage's) would be filed for both, so none is sent.
        candidate.flavour = None;
        candidate.untied_flavours = vec!["Chocolate Outrage".to_owned(), "Cola".to_owned()];
        assert!(PanelLocator::TailwindTab.serves(&candidate));
        for locator in [
            PanelLocator::NaakDrawer,
            PanelLocator::SkratchAccordion,
            PanelLocator::GuVariantLink,
        ] {
            assert!(!locator.serves(&candidate), "{locator:?}");
        }
    }

    #[tokio::test]
    async fn a_mixed_listing_is_refused_a_panel_before_its_page_is_fetched() {
        let gu = super::super::BRANDS
            .iter()
            .find(|brand| brand.slug == "gu")
            .unwrap_or_else(|| panic!("no gu brand"));
        let candidate = LabelCandidate {
            page_url: "https://guenergy.com/products/energy-gel".to_owned(),
            flavour: None,
            title: "Energy Gel".to_owned(),
            image_urls: Vec::new(),
            variant_ids: vec![31_169_146_912_819, 31_693_107_593_267],
            untied_flavours: vec!["Chocolate Outrage".to_owned(), "Cola".to_owned()],
        };
        let http = reqwest::Client::new();
        let mut pacer = PagePacer::default();
        let mut tally = Tally::default();
        let mut page = ProductPage::new(candidate.page_url.clone());
        let mut fetch = Fetcher {
            http: &http,
            pacer: &mut pacer,
            tally: &mut tally,
        };
        let found = find_panel(&mut fetch, gu, &mut page, &candidate)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let why = found.err().unwrap_or_else(|| panic!("a panel was sent"));
        assert!(
            why.contains("offers several, so no panel was sent"),
            "{why}"
        );
        assert!(matches!(page.state, PageState::Unfetched));
        assert!(pacer.last.is_empty());
        assert_eq!(
            (tally.panel_missing, tally.page_failed, tally.panel_sent),
            (1, 0, 0)
        );
    }

    #[test]
    fn only_a_family_panel_is_read_once_per_product() {
        assert!(PanelLocator::TailwindTab.per_family());
        assert!(!PanelLocator::NaakDrawer.per_family());
        assert!(!PanelLocator::SkratchAccordion.per_family());
        assert!(!PanelLocator::GuVariantLink.per_family());
    }
}
