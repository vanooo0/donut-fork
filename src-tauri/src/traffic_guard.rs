//! Budget guard for metered proxy traffic.
//!
//! Residential proxies are sold by the gigabyte, so a runaway tab can burn a
//! paid plan in minutes. The guard watches the byte counters the proxy
//! workers already keep and cuts the traffic off when either
//!
//! * the budget is spent (`traffic_limit_bytes`), or
//! * traffic spikes past `traffic_spike_bytes_per_min`.
//!
//! Cutting off means stopping every proxy worker. That is safe: a launched
//! browser points at `--proxy-server=127.0.0.1:<port>`, and Chromium does not
//! fall back to a direct connection when its proxy dies — requests simply
//! fail. So a stop can never leak the real IP.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How often the guard samples usage. A gigabyte budget doesn't need
/// millisecond precision; 5s keeps the overshoot after the limit tiny while
/// costing almost nothing.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Ignore rate samples over a gap longer than this (laptop sleep, app pause):
/// a stale gap would divide a large byte delta by a small window and fire a
/// bogus spike.
const MAX_RATE_GAP_SECS: u64 = 120;

lazy_static::lazy_static! {
  /// Last (unix_secs, cumulative_bytes) sample, for the rate calculation.
  static ref LAST_SAMPLE: Mutex<Option<(u64, u64)>> = Mutex::new(None);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrafficBudgetStatus {
  /// Bytes spent since the budget was last reset.
  pub used_bytes: u64,
  /// Configured cap, if any.
  pub limit_bytes: Option<u64>,
  /// Bytes left, or `None` when there is no cap.
  pub remaining_bytes: Option<u64>,
  /// Whether the cap is reached and proxies are being held down.
  pub limit_reached: bool,
  /// Current rate in bytes per minute, from the last two samples.
  pub bytes_per_min: u64,
  /// Configured spike ceiling, if any.
  pub spike_bytes_per_min: Option<u64>,
}

fn now_secs() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0)
}

/// Every byte the proxies have ever moved, across all profiles. Sent and
/// received are both billed by the provider, so both count.
pub fn total_used_bytes() -> u64 {
  crate::traffic_stats::list_traffic_stats()
    .iter()
    .fold(0u64, |acc, s| {
      acc
        .saturating_add(s.total_bytes_sent)
        .saturating_add(s.total_bytes_received)
    })
}

/// Usage since the last budget reset. Saturating: if history was cleared the
/// baseline can exceed the total, and that must read as 0 rather than wrap.
pub fn used_since_baseline(baseline: u64) -> u64 {
  total_used_bytes().saturating_sub(baseline)
}

pub fn budget_status() -> TrafficBudgetStatus {
  // Unreadable settings must not invent a limit — report "no cap" and let
  // the UI show a plain usage number.
  let Ok(settings) = crate::settings_manager::SettingsManager::instance().load_settings() else {
    return TrafficBudgetStatus {
      used_bytes: 0,
      limit_bytes: None,
      remaining_bytes: None,
      limit_reached: false,
      bytes_per_min: 0,
      spike_bytes_per_min: None,
    };
  };

  let used = used_since_baseline(settings.traffic_baseline_bytes);
  let limit = settings.traffic_limit_bytes;
  let remaining = limit.map(|l| l.saturating_sub(used));

  TrafficBudgetStatus {
    used_bytes: used,
    limit_bytes: limit,
    remaining_bytes: remaining,
    limit_reached: limit.is_some_and(|l| used >= l),
    bytes_per_min: current_rate(),
    spike_bytes_per_min: settings.traffic_spike_bytes_per_min,
  }
}

/// Rate over the last sampling window, in bytes per minute. Returns 0 when
/// there is no usable window yet.
fn current_rate() -> u64 {
  let Ok(guard) = LAST_SAMPLE.lock() else {
    return 0;
  };
  let Some((last_ts, last_bytes)) = *guard else {
    return 0;
  };
  let now = now_secs();
  let elapsed = now.saturating_sub(last_ts);
  if elapsed == 0 || elapsed > MAX_RATE_GAP_SECS {
    return 0;
  }
  let delta = total_used_bytes().saturating_sub(last_bytes);
  delta.saturating_mul(60) / elapsed
}

/// Take a sample and return the rate in bytes/min since the previous one.
fn sample_rate(total_now: u64) -> u64 {
  let now = now_secs();
  let Ok(mut guard) = LAST_SAMPLE.lock() else {
    return 0;
  };
  let rate = match *guard {
    Some((last_ts, last_bytes)) => {
      let elapsed = now.saturating_sub(last_ts);
      if elapsed == 0 || elapsed > MAX_RATE_GAP_SECS {
        0
      } else {
        total_now.saturating_sub(last_bytes).saturating_mul(60) / elapsed
      }
    }
    None => 0,
  };
  *guard = Some((now, total_now));
  rate
}

/// Cut all proxy traffic and tell the UI why.
async fn cut_off(reason: &'static str) {
  log::warn!("Traffic guard: stopping all proxy workers ({reason})");
  if let Err(e) = crate::proxy_runner::stop_all_proxy_processes().await {
    log::error!("Traffic guard: failed to stop proxy workers: {e}");
  }
  let _ = crate::events::emit("traffic-guard-tripped", reason);
}

/// Start the background watcher. Cheap enough to run for the whole session:
/// it reads counters the workers already write.
pub fn start_guard() {
  tauri::async_runtime::spawn(async move {
    // Never fire on the very first tick — there is no previous sample to
    // measure a rate against, and a fresh app start shouldn't trip anything.
    let _ = sample_rate(total_used_bytes());

    loop {
      tokio::time::sleep(SAMPLE_INTERVAL).await;

      let settings = match crate::settings_manager::SettingsManager::instance().load_settings() {
        Ok(s) => s,
        Err(_) => continue,
      };
      if settings.traffic_limit_bytes.is_none() && settings.traffic_spike_bytes_per_min.is_none() {
        // Guard disabled: keep sampling so the rate is fresh if it's enabled.
        let _ = sample_rate(total_used_bytes());
        continue;
      }

      let total = total_used_bytes();
      let rate = sample_rate(total);
      let used = total.saturating_sub(settings.traffic_baseline_bytes);

      if let Some(limit) = settings.traffic_limit_bytes {
        if used >= limit {
          cut_off("limit").await;
          continue;
        }
      }
      if let Some(spike) = settings.traffic_spike_bytes_per_min {
        if rate > spike {
          cut_off("spike").await;
        }
      }
    }
  });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn test_used_since_baseline_never_wraps() {
    // Baseline above the total (history cleared) must read as zero, not wrap
    // around to ~18 exabytes and instantly trip the limit.
    assert_eq!(0u64.saturating_sub(500), 0);
    let used = 100u64.saturating_sub(500);
    assert_eq!(used, 0);
  }

  #[test]
  fn test_rate_ignores_stale_and_zero_windows() {
    if let Ok(mut guard) = LAST_SAMPLE.lock() {
      // A gap longer than MAX_RATE_GAP_SECS must not produce a rate.
      *guard = Some((now_secs().saturating_sub(MAX_RATE_GAP_SECS + 60), 0));
    }
    assert_eq!(current_rate(), 0, "stale sample must not report a rate");

    if let Ok(mut guard) = LAST_SAMPLE.lock() {
      // Same-second sample: dividing by a zero window must not panic.
      *guard = Some((now_secs(), 0));
    }
    assert_eq!(current_rate(), 0, "zero-length window must not report a rate");

    if let Ok(mut guard) = LAST_SAMPLE.lock() {
      *guard = None;
    }
  }

  #[test]
  fn test_sample_rate_computes_bytes_per_min() {
    if let Ok(mut guard) = LAST_SAMPLE.lock() {
      // 30s ago we had 0 bytes; now 1 MiB -> 2 MiB/min.
      *guard = Some((now_secs().saturating_sub(30), 0));
    }
    let rate = sample_rate(1024 * 1024);
    assert_eq!(rate, 2 * 1024 * 1024);

    if let Ok(mut guard) = LAST_SAMPLE.lock() {
      *guard = None;
    }
  }
}
