//! End-to-end tests that drive the real [`ChromiumScraper`] against a local
//! `wiremock` server. Chromium must be installed on the host machine for
//! these tests to run, so they are **gated behind `ATS_E2E_CHROME=1`** and
//! skipped silently otherwise. This keeps the default CI path fast and
//! hermetic while letting a developer opt in on their workstation:
//!
//! ```powershell
//! $env:ATS_E2E_CHROME = "1"
//! cargo test -p ats-scrape --test chromium_e2e
//! ```
//!
//! The tests exercise the status-code classification matrix (happy path /
//! 401 / 403 / 404 / 451 / 418 / slow => timeout) and the reachability probe
//! for the `Offline` class. Tests that need Chromium never reach the browser
//! launch when the env gate is absent.

use std::time::Duration;

use ats_core::scrape_port::{PageScraper, ScrapeError};
use ats_scrape::ChromiumScraper;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const E2E_ENV: &str = "ATS_E2E_CHROME";

fn chrome_gate_enabled() -> bool {
    matches!(
        std::env::var(E2E_ENV).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

fn skip_unless_chrome_enabled(label: &str) -> bool {
    if chrome_gate_enabled() {
        return false;
    }
    eprintln!(
        "[ats-scrape e2e] skipping `{label}` (set {E2E_ENV}=1 with Chromium installed to run)"
    );
    true
}

fn html_ok_body() -> String {
    // Minimal HTML with a title + some sections; the scraper only returns the
    // rendered DOM, so we don't need to exercise anything dynamic here.
    r#"<!doctype html>
<html>
  <head><title>Senior Rust Engineer</title></head>
  <body>
    <h1>Senior Rust Engineer</h1>
    <section><h2>About</h2><p>Do things.</p></section>
    <section><h2>Requirements</h2><ul><li>Rust</li></ul></section>
  </body>
</html>"#
        .to_string()
}

async fn mount_status(server: &MockServer, p: &str, status: u16) {
    Mock::given(method("GET"))
        .and(path(p.to_string()))
        .respond_with(ResponseTemplate::new(status).set_body_string(format!(
            "<html><body>status {status}</body></html>"
        )))
        .mount(server)
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_offline_short_circuits_without_chrome() {
    // Probe is pure reachability — this test runs unconditionally because it
    // never touches Chromium. Exercises `ScrapeError::Offline` for an
    // unresolvable hostname.
    let scraper = ChromiumScraper::new(Duration::from_millis(500));
    let err = scraper
        .fetch_html(
            "http://ats-scrape-totally-invalid-zz-1234.invalid/",
            Duration::from_millis(500),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::Offline(_)),
        "expected Offline, got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_happy_path_returns_rendered_html() {
    if skip_unless_chrome_enabled("e2e_happy_path_returns_rendered_html") {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ok"))
        .respond_with(ResponseTemplate::new(200).set_body_string(html_ok_body()))
        .mount(&server)
        .await;

    let scraper = ChromiumScraper::new(Duration::from_secs(5));
    let html = scraper
        .fetch_html(&format!("{}/ok", server.uri()), Duration::from_secs(5))
        .await
        .expect("happy path should return rendered HTML");
    assert!(html.contains("Senior Rust Engineer"));
    assert!(html.contains("Requirements"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_auth_401_is_auth_required() {
    if skip_unless_chrome_enabled("e2e_auth_401_is_auth_required") {
        return;
    }
    let server = MockServer::start().await;
    mount_status(&server, "/auth", 401).await;

    let scraper = ChromiumScraper::new(Duration::from_secs(5));
    let err = scraper
        .fetch_html(&format!("{}/auth", server.uri()), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::AuthRequired { status: 401 }),
        "expected AuthRequired(401), got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_forbidden_403_is_auth_required() {
    if skip_unless_chrome_enabled("e2e_forbidden_403_is_auth_required") {
        return;
    }
    let server = MockServer::start().await;
    mount_status(&server, "/forbidden", 403).await;

    let scraper = ChromiumScraper::new(Duration::from_secs(5));
    let err = scraper
        .fetch_html(&format!("{}/forbidden", server.uri()), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::AuthRequired { status: 403 }),
        "expected AuthRequired(403), got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_not_found_404_is_not_found() {
    if skip_unless_chrome_enabled("e2e_not_found_404_is_not_found") {
        return;
    }
    let server = MockServer::start().await;
    mount_status(&server, "/missing", 404).await;

    let scraper = ChromiumScraper::new(Duration::from_secs(5));
    let err = scraper
        .fetch_html(&format!("{}/missing", server.uri()), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::NotFound { status: 404 }),
        "expected NotFound(404), got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_geo_451_is_geo_blocked() {
    if skip_unless_chrome_enabled("e2e_geo_451_is_geo_blocked") {
        return;
    }
    let server = MockServer::start().await;
    mount_status(&server, "/geo", 451).await;

    let scraper = ChromiumScraper::new(Duration::from_secs(5));
    let err = scraper
        .fetch_html(&format!("{}/geo", server.uri()), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::GeoBlocked { status: 451 }),
        "expected GeoBlocked(451), got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_teapot_418_is_http_status() {
    if skip_unless_chrome_enabled("e2e_teapot_418_is_http_status") {
        return;
    }
    let server = MockServer::start().await;
    mount_status(&server, "/teapot", 418).await;

    let scraper = ChromiumScraper::new(Duration::from_secs(5));
    let err = scraper
        .fetch_html(&format!("{}/teapot", server.uri()), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::Http { status: 418 }),
        "expected Http(418), got {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_slow_response_times_out() {
    if skip_unless_chrome_enabled("e2e_slow_response_times_out") {
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/slow"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(10))
                .set_body_string("<html>never</html>"),
        )
        .mount(&server)
        .await;

    // Tight idle budget forces the nav-timeout branch.
    let idle = Duration::from_millis(500);
    let scraper = ChromiumScraper::new(idle);
    let err = scraper
        .fetch_html(&format!("{}/slow", server.uri()), idle)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ScrapeError::Timeout(_)),
        "expected Timeout, got {err:?}"
    );
}
