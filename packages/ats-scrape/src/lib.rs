//! `chromiumoxide`-backed [`PageScraper`] adapter.
//!
//! [`ChromiumScraper`] is the production concretion behind
//! [`ats_core::scrape_port::PageScraper`]. It performs a reachability probe
//! before touching Chromium so offline / DNS failures are classified
//! without the cost of a browser launch.
//!
//! Platform prerequisite: a Chromium-family executable (Google Chrome,
//! Chromium, Microsoft Edge) must be installed on the host. Missing binary
//! surfaces as [`ats_core::scrape_port::ScrapeError::BrowserMissing`].

pub mod chromium;
pub mod probe;

pub use chromium::ChromiumScraper;
pub use probe::{classify_reachability_error, probe_reachability};
