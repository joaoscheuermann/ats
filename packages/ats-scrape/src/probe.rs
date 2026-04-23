//! TCP reachability probe (AC-2.5).
//!
//! Runs before launching Chromium. DNS + TCP connect, each bounded by a 2s
//! timeout. Failure is unambiguous: the host is `offline` from this client's
//! perspective. Skipped for `file://` URLs.

use std::time::Duration;

use ats_core::scrape_port::ScrapeError;
use tokio::net::{lookup_host, TcpStream};
use tokio::time::timeout;
use url::Url;

const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Return `Ok(())` if `url`'s host is reachable; otherwise a classified
/// [`ScrapeError`]. `file://` URLs bypass the probe entirely.
pub async fn probe_reachability(url: &str) -> Result<(), ScrapeError> {
    let parsed =
        Url::parse(url).map_err(|err| ScrapeError::Other(format!("invalid URL `{url}`: {err}")))?;

    if parsed.scheme() == "file" {
        return Ok(());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ScrapeError::Other(format!("URL `{url}` has no host")))?;

    let port = parsed.port_or_known_default().ok_or_else(|| {
        ScrapeError::Other(format!("URL `{url}` has no default port"))
    })?;

    let target = format!("{host}:{port}");

    let resolved = match timeout(PROBE_TIMEOUT, lookup_host(target.as_str())).await {
        Err(_) => {
            return Err(ScrapeError::Offline(format!(
                "DNS lookup for {host} timed out after {:?}",
                PROBE_TIMEOUT
            )));
        }
        Ok(Err(err)) => {
            return Err(ScrapeError::Offline(format!(
                "DNS lookup for {host} failed: {err}"
            )));
        }
        Ok(Ok(iter)) => iter.collect::<Vec<_>>(),
    };

    let first = resolved.first().ok_or_else(|| {
        ScrapeError::Offline(format!("DNS lookup for {host} returned no addresses"))
    })?;

    match timeout(PROBE_TIMEOUT, TcpStream::connect(first)).await {
        Err(_) => Err(ScrapeError::Offline(format!(
            "TCP connect to {first} timed out after {:?}",
            PROBE_TIMEOUT
        ))),
        Ok(Err(err)) => Err(ScrapeError::Offline(format!(
            "TCP connect to {first} failed: {err}"
        ))),
        Ok(Ok(_stream)) => Ok(()),
    }
}

/// Classify a navigation-level error message into [`ScrapeError::Offline`]
/// or [`ScrapeError::NetworkTimeout`]. Useful when Chromium reports a
/// Chrome-level `ERR_*` code through the CDP error string.
pub fn classify_reachability_error(err: &str) -> Option<ScrapeError> {
    let upper = err.to_uppercase();
    if upper.contains("ERR_NAME_NOT_RESOLVED")
        || upper.contains("ERR_INTERNET_DISCONNECTED")
        || upper.contains("ERR_ADDRESS_UNREACHABLE")
    {
        return Some(ScrapeError::Offline(err.to_string()));
    }
    if upper.contains("ERR_CONNECTION_REFUSED")
        || upper.contains("ERR_CONNECTION_RESET")
        || upper.contains("ERR_CONNECTION_CLOSED")
        || upper.contains("ERR_SSL_")
        || upper.contains("ERR_CERT_")
    {
        return Some(ScrapeError::NetworkTimeout(err.to_string()));
    }
    if upper.contains("ERR_TIMED_OUT") {
        return Some(ScrapeError::NetworkTimeout(err.to_string()));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn unresolvable_host_returns_offline() {
        let err = probe_reachability("http://definitely-not-a-real-host-zzz12345.invalid")
            .await
            .unwrap_err();
        match err {
            ScrapeError::Offline(_) => {}
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reachable_loopback_returns_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}/path", addr.port());
        probe_reachability(&url).await.expect("loopback is reachable");
    }

    #[tokio::test]
    async fn refused_connection_returns_offline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let url = format!("http://127.0.0.1:{port}/");
        let err = probe_reachability(&url).await.unwrap_err();
        match err {
            ScrapeError::Offline(_) => {}
            other => panic!("expected Offline, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_url_returns_other() {
        let err = probe_reachability("not a url").await.unwrap_err();
        match err {
            ScrapeError::Other(_) => {}
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn file_urls_bypass_probe() {
        probe_reachability("file:///tmp/does/not/matter").await.unwrap();
    }

    #[test]
    fn classify_offline_strings() {
        assert!(matches!(
            classify_reachability_error("Chrome error: net::ERR_NAME_NOT_RESOLVED"),
            Some(ScrapeError::Offline(_))
        ));
        assert!(matches!(
            classify_reachability_error("net::ERR_INTERNET_DISCONNECTED"),
            Some(ScrapeError::Offline(_))
        ));
    }

    #[test]
    fn classify_transport_strings() {
        assert!(matches!(
            classify_reachability_error("net::ERR_CONNECTION_REFUSED"),
            Some(ScrapeError::NetworkTimeout(_))
        ));
        assert!(matches!(
            classify_reachability_error("net::ERR_TIMED_OUT"),
            Some(ScrapeError::NetworkTimeout(_))
        ));
    }

    #[test]
    fn classify_unknown_returns_none() {
        assert!(classify_reachability_error("Some unrelated error").is_none());
    }
}
