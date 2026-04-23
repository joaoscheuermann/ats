//! Page-scraper port trait and its error taxonomy.
//!
//! Kept in its own module so the `PageScraper` surface stays narrow (ISP)
//! and so the adapter crate (`ats-scrape`) can depend on just this sliver
//! of `ats-core`. Each [`ScrapeError`] variant maps 1:1 to a
//! [`crate::error::ScrapeClass`] — the mapping lives here so every caller
//! agrees on what a given adapter-level failure means at the CLI level.

use std::time::Duration;

use async_trait::async_trait;

use crate::error::ScrapeClass;

/// Fetches rendered HTML for a URL. The concretion (`ChromiumScraper`) lives
/// in the `ats-scrape` crate; pipeline stages depend only on the trait.
#[async_trait]
pub trait PageScraper: Send + Sync {
    /// Navigate to `url`, wait up to `idle_timeout` for the network to
    /// settle, and return the rendered HTML. Errors must map cleanly to
    /// one of the [`ScrapeError`] variants so the stage doesn't have to
    /// guess.
    async fn fetch_html(
        &self,
        url: &str,
        idle_timeout: Duration,
    ) -> Result<String, ScrapeError>;
}

/// Adapter-level scrape failure taxonomy. Production adapters (and the
/// stage) translate into [`crate::error::AtsError::Scrape`] (exit code 4)
/// via [`From`].
#[derive(Debug, Clone, thiserror::Error)]
pub enum ScrapeError {
    /// Reachability probe failed (DNS or TCP connect). Short-circuits
    /// before Chromium is launched.
    #[error("offline: {0}")]
    Offline(String),
    /// Transport-level failure mid-navigation (e.g. connection reset,
    /// TLS failure).
    #[error("network-timeout: {0}")]
    NetworkTimeout(String),
    /// The network-idle waiter timed out while the page was still busy.
    #[error("scrape/timeout: {0}")]
    Timeout(String),
    /// Main-frame 401/403.
    #[error("auth-required (status {status})")]
    AuthRequired { status: u16 },
    /// Main-frame 404.
    #[error("not-found (status {status})")]
    NotFound { status: u16 },
    /// Main-frame 451.
    #[error("geo-blocked (status {status})")]
    GeoBlocked { status: u16 },
    /// Any other non-2xx/3xx main-frame status.
    #[error("http {status}")]
    Http { status: u16 },
    /// `chromiumoxide` could not find a Chromium-family executable.
    #[error("browser-missing: {0}")]
    BrowserMissing(String),
    /// Anything else the adapter wants to surface.
    #[error("other: {0}")]
    Other(String),
}

impl From<ScrapeError> for ScrapeClass {
    fn from(err: ScrapeError) -> Self {
        match err {
            ScrapeError::Offline(_) => ScrapeClass::Offline,
            ScrapeError::NetworkTimeout(_) => ScrapeClass::NetworkTimeout,
            ScrapeError::Timeout(_) => ScrapeClass::Timeout,
            ScrapeError::AuthRequired { .. } => ScrapeClass::AuthRequired,
            ScrapeError::NotFound { .. } => ScrapeClass::NotFound,
            ScrapeError::GeoBlocked { .. } => ScrapeClass::GeoBlocked,
            ScrapeError::Http { status } => ScrapeClass::Http(status),
            ScrapeError::BrowserMissing(msg) => ScrapeClass::BrowserMissing(msg),
            ScrapeError::Other(msg) => ScrapeClass::Other(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrape_error_maps_to_expected_class() {
        let cases: &[(ScrapeError, ScrapeClass)] = &[
            (ScrapeError::Offline("dns".into()), ScrapeClass::Offline),
            (
                ScrapeError::NetworkTimeout("reset".into()),
                ScrapeClass::NetworkTimeout,
            ),
            (
                ScrapeError::Timeout("idle".into()),
                ScrapeClass::Timeout,
            ),
            (
                ScrapeError::AuthRequired { status: 401 },
                ScrapeClass::AuthRequired,
            ),
            (
                ScrapeError::NotFound { status: 404 },
                ScrapeClass::NotFound,
            ),
            (
                ScrapeError::GeoBlocked { status: 451 },
                ScrapeClass::GeoBlocked,
            ),
            (
                ScrapeError::Http { status: 418 },
                ScrapeClass::Http(418),
            ),
        ];
        for (err, expected) in cases {
            let class: ScrapeClass = err.clone().into();
            assert_eq!(class, *expected, "{err:?}");
        }
    }

    #[test]
    fn class_tag_is_stable_for_browser_missing_and_other() {
        assert_eq!(
            ScrapeClass::from(ScrapeError::BrowserMissing("Chrome gone".into())).class_tag(),
            "browser-missing"
        );
        assert_eq!(
            ScrapeClass::from(ScrapeError::Other("something".into())).class_tag(),
            "other"
        );
    }

    #[test]
    fn http_class_tag_embeds_status() {
        assert_eq!(
            ScrapeClass::from(ScrapeError::Http { status: 418 }).class_tag(),
            "http-418"
        );
    }
}
