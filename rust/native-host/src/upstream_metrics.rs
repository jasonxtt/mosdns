//! The per-upstream series of `GET /metrics` for `aliapi` groups, in the
//! reference's Prometheus text format: `mosdns_aliapi_{error_total,query_total,
//! response_latency_millisecond,upstream_winner_total}` labelled `addr`,
//! `metrics_tag` (the plugin tag) and `tag` (the upstream tag). As in the
//! reference, only upstreams that carry a `tag` have series, and the series
//! exist (at zero) from startup.
//!
//! Not emitted, because the native runtime has no matching event: the
//! `inflight_queries` gauge and the `conn_opened_total` / `conn_closed_total`
//! counters (see the native WebUI hosting contract).

use std::fmt::Write;

use crate::config::CompiledConfig;
use crate::observer::{
    MetricsSnapshot, UPSTREAM_LATENCY_BOUNDS_MS, UpstreamAttemptMetricsSnapshot,
};

/// One tagged `aliapi` upstream and its counters.
pub(crate) struct Series<'a> {
    pub(crate) addr: &'a str,
    pub(crate) metrics_tag: &'a str,
    pub(crate) tag: &'a str,
    pub(crate) counters: UpstreamAttemptMetricsSnapshot,
}

/// The series of every tagged `aliapi` upstream in `config`.
pub(crate) fn series<'a>(
    config: &'a CompiledConfig,
    snapshot: &MetricsSnapshot,
) -> Vec<Series<'a>> {
    let mut series = Vec::new();
    for definition in &config.forward_definitions {
        for entry in &definition.entries {
            let (Some(tag), Some(addr)) = (entry.tag.as_deref(), entry.metrics_addr.as_deref())
            else {
                continue;
            };
            series.push(Series {
                addr,
                metrics_tag: &definition.tag,
                tag,
                counters: snapshot
                    .forward_attempts_by_upstream
                    .get(&entry.identity)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }
    series
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn labels(series: &Series<'_>, le: Option<&str>) -> String {
    let mut text = format!(
        "addr=\"{}\",metrics_tag=\"{}\",tag=\"{}\"",
        escape(series.addr),
        escape(series.metrics_tag),
        escape(series.tag)
    );
    if let Some(le) = le {
        let _ = write!(text, ",le=\"{le}\"");
    }
    text
}

/// Appends the families (alphabetical, series sorted by label values as the
/// reference's gatherer does). Nothing is written without series.
pub(crate) fn render(mut series: Vec<Series<'_>>, out: &mut String) {
    if series.is_empty() {
        return;
    }
    series.sort_by(|a, b| (a.addr, a.metrics_tag, a.tag).cmp(&(b.addr, b.metrics_tag, b.tag)));
    let counter = |out: &mut String, name: &str, help: &str, value: &dyn Fn(&Series<'_>) -> u64| {
        let _ = writeln!(out, "# HELP mosdns_aliapi_{name} {help}");
        let _ = writeln!(out, "# TYPE mosdns_aliapi_{name} counter");
        for item in &series {
            let _ = writeln!(
                out,
                "mosdns_aliapi_{name}{{{}}} {}",
                labels(item, None),
                value(item)
            );
        }
    };
    counter(
        out,
        "error_total",
        "Total number of query errors.",
        &|item| {
            item.counters
                .failures_total
                .saturating_add(item.counters.timeouts_total)
        },
    );
    counter(out, "query_total", "Total number of queries.", &|item| {
        item.counters.attempts_total
    });
    let _ = writeln!(
        out,
        "# HELP mosdns_aliapi_response_latency_millisecond Response latency in milliseconds."
    );
    let _ = writeln!(
        out,
        "# TYPE mosdns_aliapi_response_latency_millisecond histogram"
    );
    for item in &series {
        let mut cumulative = 0_u64;
        for (index, bound) in UPSTREAM_LATENCY_BOUNDS_MS.iter().enumerate() {
            cumulative = cumulative.saturating_add(item.counters.latency_buckets[index]);
            let _ = writeln!(
                out,
                "mosdns_aliapi_response_latency_millisecond_bucket{{{}}} {cumulative}",
                labels(item, Some(&bound.to_string()))
            );
        }
        let _ = writeln!(
            out,
            "mosdns_aliapi_response_latency_millisecond_bucket{{{}}} {}",
            labels(item, Some("+Inf")),
            item.counters.latency_count
        );
        let _ = writeln!(
            out,
            "mosdns_aliapi_response_latency_millisecond_sum{{{}}} {}",
            labels(item, None),
            item.counters.latency_sum_ms
        );
        let _ = writeln!(
            out,
            "mosdns_aliapi_response_latency_millisecond_count{{{}}} {}",
            labels(item, None),
            item.counters.latency_count
        );
    }
    counter(
        out,
        "upstream_winner_total",
        "Total number of times this upstream result was selected as the final response.",
        &|item| item.counters.winners_total,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters() -> UpstreamAttemptMetricsSnapshot {
        let mut latency_buckets = [0; UPSTREAM_LATENCY_BOUNDS_MS.len() + 1];
        latency_buckets[0] = 1; // <= 1 ms
        latency_buckets[3] = 1; // <= 20 ms
        latency_buckets[11] = 1; // > 5000 ms
        UpstreamAttemptMetricsSnapshot {
            attempts_total: 6,
            responses_total: 3,
            failures_total: 1,
            timeouts_total: 1,
            canceled_total: 1,
            winners_total: 2,
            latency_buckets,
            latency_sum_ms: 6015,
            latency_count: 3,
            ..UpstreamAttemptMetricsSnapshot::default()
        }
    }

    #[test]
    fn families_follow_the_reference_names_labels_and_histogram_shape() {
        let mut text = String::new();
        render(
            vec![
                Series {
                    addr: "udp://9.9.9.9",
                    metrics_tag: "foreign",
                    tag: "b\"q\\x",
                    counters: counters(),
                },
                Series {
                    addr: "https://dns.alidns.com/dns-query",
                    metrics_tag: "domestic",
                    tag: "ali",
                    counters: UpstreamAttemptMetricsSnapshot::default(),
                },
            ],
            &mut text,
        );
        let series = "addr=\"udp://9.9.9.9\",metrics_tag=\"foreign\",tag=\"b\\\"q\\\\x\"";
        // error = failures + timeouts; query = every attempt (the canceled one too).
        for (name, value) in [
            ("error_total", 2),
            ("query_total", 6),
            ("upstream_winner_total", 2),
        ] {
            assert!(
                text.contains(&format!("mosdns_aliapi_{name}{{{series}}} {value}\n")),
                "{name}\n{text}"
            );
        }
        for (le, count) in [
            ("1", 1),
            ("5", 1),
            ("10", 1),
            ("20", 2),
            ("5000", 2),
            ("+Inf", 3),
        ] {
            assert!(
                text.contains(&format!(
                    "mosdns_aliapi_response_latency_millisecond_bucket{{{series},le=\"{le}\"}} {count}\n"
                )),
                "le={le}\n{text}"
            );
        }
        assert!(text.contains(&format!(
            "mosdns_aliapi_response_latency_millisecond_sum{{{series}}} 6015\n"
        )));
        assert!(text.contains(&format!(
            "mosdns_aliapi_response_latency_millisecond_count{{{series}}} 3\n"
        )));
        // Series are sorted by label values; zero-valued upstreams still appear.
        let ali = text.find("addr=\"https://dns.alidns.com").unwrap();
        let udp = text.find("addr=\"udp://9.9.9.9\"").unwrap();
        assert!(ali < udp);
        assert!(text.contains("# TYPE mosdns_aliapi_response_latency_millisecond histogram\n"));
        assert!(!text.contains("inflight_queries") && !text.contains("conn_opened_total"));
    }

    #[test]
    fn no_upstreams_means_no_output() {
        let mut text = String::new();
        render(Vec::new(), &mut text);
        assert!(text.is_empty());
    }
}
