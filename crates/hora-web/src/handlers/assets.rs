//! The compiled-in static assets: the one stylesheet, the brand fonts, the
//! mark and the favicon. Every page links them with a `?v=` content hash,
//! so they are served `immutable`: a new build changes the hash, never the
//! bytes behind an old URL.

use std::sync::LazyLock;

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

/// The favicon, also served at the conventional `/favicon.svg`.
pub(crate) const FAVICON_SVG: &str = include_str!("../../assets/favicon.svg");
const MARK_SVG: &str = include_str!("../../assets/mark.svg");
/// The stylesheet as written; `__V__` stands for [`ASSET_VERSION`] in its
/// font URLs, so a font change busts the stylesheet's cache too.
const CSS_SOURCE: &str = include_str!("../../assets/hora.css");

const FRAUNCES: &[u8] = include_bytes!("../../assets/fonts/Fraunces-VF.woff2");
const CAL_SANS: &[u8] = include_bytes!("../../assets/fonts/CalSansVF.woff2");
const BOREL: &[u8] = include_bytes!("../../assets/fonts/BorelDisplay-Regular.woff2");
/// Mona Sans Mono: "Mona" is a Reserved Font Name under the OFL, so the file
/// ships exactly as released (not subset, converted or renamed).
const MONA_SANS_MONO: &[u8] = include_bytes!("../../assets/fonts/MonaSansMono.woff2");

/// Every binary asset by its path under `/assets/`, with its media type.
const BINARY: &[(&str, &[u8], &str)] = &[
    ("fonts/Fraunces-VF.woff2", FRAUNCES, "font/woff2"),
    ("fonts/CalSansVF.woff2", CAL_SANS, "font/woff2"),
    ("fonts/BorelDisplay-Regular.woff2", BOREL, "font/woff2"),
    ("fonts/MonaSansMono.woff2", MONA_SANS_MONO, "font/woff2"),
    ("mark.svg", MARK_SVG.as_bytes(), "image/svg+xml"),
    ("favicon.svg", FAVICON_SVG.as_bytes(), "image/svg+xml"),
];

/// A short hash of every asset's bytes: the `?v=` cache buster. FNV-1a, so
/// it is stable across builds and toolchains (std's hasher is not).
pub(crate) static ASSET_VERSION: LazyLock<String> = LazyLock::new(|| {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let sources = BINARY
        .iter()
        .map(|(_, bytes, _)| *bytes)
        .chain(std::iter::once(CSS_SOURCE.as_bytes()));
    for bytes in sources {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{:08x}", hash >> 32)
});

/// The served stylesheet: the source with its version placeholder filled.
static CSS: LazyLock<String> = LazyLock::new(|| CSS_SOURCE.replace("__V__", &ASSET_VERSION));

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// `GET /assets/{*path}`: a compiled-in asset, or 404.
pub(crate) async fn asset(Path(path): Path<String>) -> Response {
    if path == "hora.css" {
        return (
            [
                (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                (header::CACHE_CONTROL, IMMUTABLE),
            ],
            CSS.as_str(),
        )
            .into_response();
    }
    match BINARY.iter().find(|(name, _, _)| *name == path) {
        Some((_, bytes, kind)) => (
            [
                (header::CONTENT_TYPE, *kind),
                (header::CACHE_CONTROL, IMMUTABLE),
            ],
            *bytes,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "unknown asset").into_response(),
    }
}

/// `GET /favicon.svg`, where browsers look first. Not versioned (the URL is
/// fixed by convention), so cached for a day only.
pub(crate) async fn favicon() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        FAVICON_SVG,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_short_hex_and_fills_the_stylesheet() {
        assert_eq!(ASSET_VERSION.len(), 8);
        assert!(ASSET_VERSION.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!CSS.contains("__V__"));
        assert!(CSS.contains(&format!("CalSansVF.woff2?v={}", *ASSET_VERSION)));
    }

    #[test]
    fn stylesheet_needs_no_inline_style_or_remote_origin() {
        // Fonts are same-origin (CSP font-src 'self'); nothing is fetched elsewhere.
        assert!(!CSS.contains("http://") && !CSS.contains("https://"));
    }
}
