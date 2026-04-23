//! [`PageScraper`] implementation backed by `chromiumoxide`.
//!
//! One browser process per `fetch_html` call — the CLI handles a single URL
//! per invocation (NFC-18), so keeping the lifecycle scoped to the call
//! keeps the concurrency model trivially simple and makes resource cleanup
//! easy via RAII.
//!
//! # Classification strategy
//!
//! We subscribe to `Network.responseReceived` **before** navigating and
//! watch for the first `ResourceType::Document` event on the main frame.
//! That event's HTTP status decides the `auth-required | not-found |
//! geo-blocked | http <status>` branch. Chrome transport-level errors
//! (`ERR_NAME_NOT_RESOLVED`, etc.) are classified from the navigation
//! error's rendered string by [`crate::probe::classify_reachability_error`].
//!
//! # Resource safety
//!
//! [`BrowserGuard`] owns the browser handle and the handler task join
//! handle. Dropping it aborts the event loop and spawns a best-effort
//! close; every early-return path therefore releases Chromium.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::network::{
    EventResponseReceived, ResourceType,
};
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::viewport::Viewport;
use futures::StreamExt;
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use ats_core::scrape_port::{PageScraper, ScrapeError};

use crate::probe::{classify_reachability_error, probe_reachability};

/// Soft ceiling on how long we wait for `Browser::launch` to complete. If
/// Chromium doesn't hand back a handle inside this, it's usually missing or
/// broken; we fail fast rather than hang the CLI.
const BROWSER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Number of extra idle millis we give the page after `wait_for_navigation`
/// resolves, on top of the configured idle timeout. Lets SPAs finish a final
/// XHR burst before we snapshot the DOM.
const POST_NAV_GRACE: Duration = Duration::from_millis(500);

/// Upper bound on the graceful browser-shutdown path. chromiumoxide's
/// `page.close()` / `browser.close()` / `browser.wait()` round-trip through
/// CDP; on Windows with a chromiumoxide whose generated `Message` enum
/// drifts against current Chrome, the handler can drop shutdown replies as
/// "invalid message" and the close futures sit forever. We already have the
/// HTML in hand at this point, so it's safe to abandon the graceful close
/// and let [`BrowserGuard::drop`] finish it best-effort in the background.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// Default viewport — generous enough that most postings render without
/// mobile-layout quirks.
const DEFAULT_VIEWPORT_WIDTH: u32 = 1280;
const DEFAULT_VIEWPORT_HEIGHT: u32 = 800;

/// Production [`PageScraper`]. Cheap to construct — the browser is only
/// spawned inside [`PageScraper::fetch_html`].
#[derive(Debug, Default, Clone)]
pub struct ChromiumScraper {
    #[allow(dead_code)]
    idle_timeout_hint: Option<Duration>,
}

impl ChromiumScraper {
    /// Create a new scraper. `idle_timeout_hint` is informational; the
    /// binding [`PageScraper::fetch_html`] call supplies the real timeout.
    pub fn new(idle_timeout_hint: Duration) -> Self {
        Self {
            idle_timeout_hint: Some(idle_timeout_hint),
        }
    }
}

#[async_trait]
impl PageScraper for ChromiumScraper {
    async fn fetch_html(
        &self,
        url: &str,
        idle_timeout: Duration,
    ) -> Result<String, ScrapeError> {
        let started = Instant::now();
        info!(target: "ats::scrape", url, idle_timeout_ms = idle_timeout.as_millis() as u64, "scrape.probe.start");
        probe_reachability(url).await?;
        info!(target: "ats::scrape", elapsed_ms = started.elapsed().as_millis() as u64, "scrape.probe.ok");

        info!(target: "ats::scrape", "scrape.browser.launching");
        let launch_started = Instant::now();
        let mut guard = launch_browser().await?;
        info!(target: "ats::scrape", elapsed_ms = launch_started.elapsed().as_millis() as u64, "scrape.browser.launched");

        let page = guard
            .browser
            .as_ref()
            .expect("browser just launched")
            .new_page("about:blank")
            .await
            .map_err(|err| map_navigation_error(&err))?;

        let status_slot: Arc<Mutex<Option<u16>>> = Arc::new(Mutex::new(None));
        let listener_task = spawn_response_listener(&page, status_slot.clone()).await?;

        let nav_budget = idle_timeout + POST_NAV_GRACE;
        info!(target: "ats::scrape", url, budget_ms = nav_budget.as_millis() as u64, "scrape.navigation.start");
        let nav_started = Instant::now();
        let nav_result = timeout(nav_budget, async {
            page.goto(url).await?;
            page.wait_for_navigation().await?;
            Ok::<(), CdpError>(())
        })
        .await;

        match nav_result {
            Err(_elapsed) => {
                listener_task.abort();
                shutdown_or_abandon(&mut guard, Some(page)).await;
                return Err(ScrapeError::Timeout(format!(
                    "page did not finish navigating within {nav_budget:?}"
                )));
            }
            Ok(Err(err)) => {
                listener_task.abort();
                shutdown_or_abandon(&mut guard, Some(page)).await;
                return Err(map_navigation_error(&err));
            }
            Ok(Ok(())) => {
                info!(
                    target: "ats::scrape",
                    elapsed_ms = nav_started.elapsed().as_millis() as u64,
                    "scrape.navigation.complete"
                );
            }
        }

        tokio::time::sleep(POST_NAV_GRACE).await;

        let status = *status_slot.lock().expect("status slot poisoned");
        if let Some(status) = status {
            info!(target: "ats::scrape", status, "scrape.main_frame.status");
            if let Some(err) = classify_status(status) {
                listener_task.abort();
                shutdown_or_abandon(&mut guard, Some(page)).await;
                return Err(err);
            }
        } else {
            warn!(
                target: "ats::scrape",
                url,
                "no main-frame response captured; proceeding with rendered content"
            );
        }

        info!(target: "ats::scrape", "scrape.content.capturing");
        let capture_started = Instant::now();
        let content = match page.content().await {
            Ok(html) => html,
            Err(err) => {
                listener_task.abort();
                shutdown_or_abandon(&mut guard, Some(page)).await;
                return Err(map_navigation_error(&err));
            }
        };
        info!(
            target: "ats::scrape",
            bytes = content.len(),
            capture_elapsed_ms = capture_started.elapsed().as_millis() as u64,
            total_elapsed_ms = started.elapsed().as_millis() as u64,
            "scrape.html.captured"
        );

        listener_task.abort();
        shutdown_or_abandon(&mut guard, Some(page)).await;
        Ok(content)
    }
}

/// Best-effort graceful close of the page + browser, bounded by
/// [`SHUTDOWN_TIMEOUT`]. If either step times out, we leave cleanup to
/// [`BrowserGuard::drop`] so we never block the pipeline on a stuck CDP
/// round-trip. Emits breadcrumbs so operators can see which step of the
/// shutdown hung (without enabling DEBUG on chromiumoxide itself).
async fn shutdown_or_abandon(
    guard: &mut BrowserGuard,
    page: Option<chromiumoxide::Page>,
) {
    if let Some(page) = page {
        match timeout(SHUTDOWN_TIMEOUT, page.close()).await {
            Ok(result) => {
                if let Err(err) = result {
                    debug!(target: "ats::scrape", %err, "page.close returned error");
                }
            }
            Err(_) => warn!(
                target: "ats::scrape",
                timeout_ms = SHUTDOWN_TIMEOUT.as_millis() as u64,
                "scrape.page.close.timeout"
            ),
        }
    }

    match timeout(SHUTDOWN_TIMEOUT, guard.shutdown()).await {
        Ok(()) => info!(target: "ats::scrape", "scrape.browser.shutdown.ok"),
        Err(_) => warn!(
            target: "ats::scrape",
            timeout_ms = SHUTDOWN_TIMEOUT.as_millis() as u64,
            "scrape.browser.shutdown.timeout"
        ),
    }
}

/// RAII wrapper that closes the browser and aborts the event-loop task on
/// drop, so every early-return path releases Chromium. Prefer calling
/// [`BrowserGuard::shutdown`] on the happy path so the close future is
/// actually awaited; [`Drop`] is the escape hatch for panics and early
/// returns.
struct BrowserGuard {
    _profile: Option<TempDir>,
    browser: Option<Browser>,
    handler_task: Option<JoinHandle<()>>,
}

impl BrowserGuard {
    async fn shutdown(&mut self) {
        if let Some(task) = self.handler_task.take() {
            task.abort();
        }
        if let Some(mut browser) = self.browser.take() {
            let _ = browser.close().await;
            let _ = browser.wait().await;
        }
    }
}

impl Drop for BrowserGuard {
    fn drop(&mut self) {
        if let Some(task) = self.handler_task.take() {
            task.abort();
        }
        if let Some(mut browser) = self.browser.take() {
            // Best-effort background close: Drop cannot await.
            tokio::spawn(async move {
                let _ = browser.close().await;
                let _ = browser.wait().await;
            });
        }
    }
}

async fn launch_browser() -> Result<BrowserGuard, ScrapeError> {
    let profile = tempfile::tempdir().map_err(|e| {
        ScrapeError::Other(format!("browser profile temp dir: {e}"))
    })?;

    let mut builder = BrowserConfig::builder()
        .user_data_dir(profile.path())
        .viewport(Some(Viewport {
            width: DEFAULT_VIEWPORT_WIDTH,
            height: DEFAULT_VIEWPORT_HEIGHT,
            ..Viewport::default()
        }));
    #[cfg(windows)]
    {
        builder = builder.no_sandbox();
    }

    let config = builder
        .build()
        .map_err(ScrapeError::BrowserMissing)?;

    let launched = timeout(BROWSER_LAUNCH_TIMEOUT, Browser::launch(config)).await;
    let (browser, mut handler) = match launched {
        Err(_) => {
            return Err(ScrapeError::BrowserMissing(format!(
                "Chromium did not launch within {BROWSER_LAUNCH_TIMEOUT:?}"
            )))
        }
        Ok(Err(err)) => return Err(map_launch_error(err)),
        Ok(Ok(pair)) => pair,
    };

    let handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if let Err(err) = event {
                debug!(target: "ats::scrape", %err, "chromium handler event error");
            }
        }
    });

    Ok(BrowserGuard {
        _profile: Some(profile),
        browser: Some(browser),
        handler_task: Some(handler_task),
    })
}

/// Decide whether a [`CdpError`] from `Browser::launch` indicates a missing
/// Chromium binary. chromiumoxide surfaces this as `LaunchExit`, `LaunchIo`,
/// or a Chrome message referencing the launcher.
fn map_launch_error(err: CdpError) -> ScrapeError {
    let msg = err.to_string();
    let lower = msg.to_lowercase();
    let browser_missing_markers = [
        "could not auto detect",
        "chrome executable",
        "no such file",
        "cannot find",
        "not found",
        "chromiumlauncher",
        "executable",
    ];
    if browser_missing_markers
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return ScrapeError::BrowserMissing(msg);
    }
    match err {
        CdpError::LaunchExit(_, _)
        | CdpError::LaunchIo(_, _)
        | CdpError::LaunchTimeout(_) => ScrapeError::BrowserMissing(msg),
        _ => ScrapeError::Other(msg),
    }
}

async fn spawn_response_listener(
    page: &chromiumoxide::Page,
    status_slot: Arc<Mutex<Option<u16>>>,
) -> Result<JoinHandle<()>, ScrapeError> {
    let mut stream = page
        .event_listener::<EventResponseReceived>()
        .await
        .map_err(|err| map_navigation_error(&err))?;

    let main_frame = page
        .mainframe()
        .await
        .map_err(|err| map_navigation_error(&err))?;

    Ok(tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            if event.r#type != ResourceType::Document {
                continue;
            }
            if event.frame_id.as_ref() != main_frame.as_ref() {
                continue;
            }
            let status = event.response.status as u16;
            let mut guard = match status_slot.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            if guard.is_none() {
                *guard = Some(status);
            }
        }
    }))
}

fn classify_status(status: u16) -> Option<ScrapeError> {
    match status {
        0 => None,
        200..=399 => None,
        401 | 403 => Some(ScrapeError::AuthRequired { status }),
        404 => Some(ScrapeError::NotFound { status }),
        451 => Some(ScrapeError::GeoBlocked { status }),
        _ => Some(ScrapeError::Http { status }),
    }
}

fn map_navigation_error(err: &CdpError) -> ScrapeError {
    let msg = err.to_string();
    if let Some(classified) = classify_reachability_error(&msg) {
        return classified;
    }
    if matches!(err, CdpError::Timeout) {
        return ScrapeError::Timeout(msg);
    }
    ScrapeError::Other(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_status_maps_locked_buckets() {
        assert!(classify_status(200).is_none());
        assert!(classify_status(301).is_none());
        assert!(matches!(
            classify_status(401),
            Some(ScrapeError::AuthRequired { status: 401 })
        ));
        assert!(matches!(
            classify_status(403),
            Some(ScrapeError::AuthRequired { status: 403 })
        ));
        assert!(matches!(
            classify_status(404),
            Some(ScrapeError::NotFound { status: 404 })
        ));
        assert!(matches!(
            classify_status(451),
            Some(ScrapeError::GeoBlocked { status: 451 })
        ));
        assert!(matches!(
            classify_status(418),
            Some(ScrapeError::Http { status: 418 })
        ));
        assert!(matches!(
            classify_status(500),
            Some(ScrapeError::Http { status: 500 })
        ));
    }

    #[test]
    fn map_launch_error_recognises_missing_chrome_text() {
        let err = CdpError::ChromeMessage("Could not auto detect a Chrome executable".into());
        match map_launch_error(err) {
            ScrapeError::BrowserMissing(msg) => {
                assert!(msg.to_lowercase().contains("auto detect"))
            }
            other => panic!("expected BrowserMissing, got {other:?}"),
        }
    }

    #[test]
    fn map_navigation_error_prefers_chrome_net_classification() {
        let err = CdpError::ChromeMessage("net::ERR_NAME_NOT_RESOLVED".into());
        assert!(matches!(
            map_navigation_error(&err),
            ScrapeError::Offline(_)
        ));
        let err = CdpError::ChromeMessage("net::ERR_CONNECTION_REFUSED".into());
        assert!(matches!(
            map_navigation_error(&err),
            ScrapeError::NetworkTimeout(_)
        ));
    }

    #[test]
    fn map_navigation_error_cdp_timeout_is_scrape_timeout() {
        assert!(matches!(
            map_navigation_error(&CdpError::Timeout),
            ScrapeError::Timeout(_)
        ));
    }
}
