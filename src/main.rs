use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

// =============================================================================
// CLI
// =============================================================================

#[derive(Parser)]
#[command(name = "k6r")]
#[command(version)]
#[command(about = "Convert K6 handleSummary JSON to Markdown reports")]
struct Cli {
    /// Input K6 JSON summary file
    #[arg(value_name = "JSON_FILE")]
    input: PathBuf,

    /// Output Markdown file (defaults to input filename with .md extension)
    #[arg(value_name = "MARKDOWN_FILE")]
    output: Option<PathBuf>,
}

// =============================================================================
// Data Model
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct K6Summary {
    pub metrics: HashMap<String, Metric>,
    pub root_group: Option<Group>,
    pub state: Option<State>,
}

#[derive(Debug, Deserialize)]
pub struct Metric {
    #[serde(rename = "type")]
    pub metric_type: MetricType,
    pub contains: String,
    pub values: HashMap<String, f64>,
    #[serde(default)]
    pub thresholds: HashMap<String, Threshold>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Counter,
    Rate,
    Gauge,
    Trend,
}

#[derive(Debug, Deserialize)]
pub struct Threshold {
    pub ok: bool,
}

#[derive(Debug, Deserialize)]
pub struct Group {
    pub name: String,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub checks: Vec<Check>,
}

#[derive(Debug, Deserialize)]
pub struct Check {
    pub name: String,
    pub passes: u64,
    pub fails: u64,
}

#[derive(Debug, Deserialize)]
pub struct State {
    #[serde(rename = "testRunDurationMs")]
    pub test_run_duration_ms: f64,
}

// =============================================================================
// Formatting Utilities
// =============================================================================

fn format_duration(ms: f64) -> String {
    if ms >= 60_000.0 {
        let mins = ms / 60_000.0;
        format!("{:.2}m", mins)
    } else if ms >= 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else if ms >= 1.0 {
        format!("{:.2}ms", ms)
    } else {
        format!("{:.2}µs", ms * 1000.0)
    }
}

fn format_count(count: f64) -> String {
    let count = count as u64;
    if count >= 1_000_000 {
        format!("{:.2}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{:.2}K", count as f64 / 1_000.0)
    } else {
        format!("{}", count)
    }
}

fn format_rate(rate: f64) -> String {
    format!("{:.2}/s", rate)
}

fn format_percent(rate: f64) -> String {
    format!("{:.2}%", rate * 100.0)
}

fn format_value(value: f64, key: &str, contains: &str, metric_type: MetricType) -> String {
    if contains == "time" {
        format_duration(value)
    } else if key == "rate" {
        // Counter의 rate는 /s, Rate 메트릭의 rate는 %
        match metric_type {
            MetricType::Counter => format_rate(value),
            MetricType::Rate => format_percent(value),
            _ => format!("{:.2}", value),
        }
    } else if key == "count" || key == "passes" || key == "fails" {
        format_count(value)
    } else {
        format!("{:.2}", value)
    }
}

// =============================================================================
// Report Generation
// =============================================================================

fn generate_report(summary: &K6Summary) -> String {
    let mut output = String::with_capacity(8192);

    // Header
    output.push_str("# K6 Load Test Report\n\n");

    // Test duration
    if let Some(state) = &summary.state {
        output.push_str(&format!(
            "**Test Duration:** {}\n\n",
            format_duration(state.test_run_duration_ms)
        ));
    }

    output.push_str("---\n\n");

    // Summary section
    output.push_str(&generate_summary_section(summary));

    // Thresholds section
    output.push_str(&generate_thresholds_section(summary));

    // HTTP Metrics section
    output.push_str(&generate_http_metrics_section(summary));

    // Checks section
    output.push_str(&generate_checks_section(summary));

    // All metrics section
    output.push_str(&generate_all_metrics_section(summary));

    output
}

fn generate_summary_section(summary: &K6Summary) -> String {
    let mut output = String::new();
    output.push_str("## Summary\n\n");
    output.push_str("| Metric | Value |\n");
    output.push_str("|--------|-------|\n");

    // Total requests
    if let Some(metric) = summary.metrics.get("http_reqs") {
        if let Some(count) = metric.values.get("count") {
            output.push_str(&format!("| Total Requests | {} |\n", format_count(*count)));
        }
        if let Some(rate) = metric.values.get("rate") {
            output.push_str(&format!("| Request Rate | {} |\n", format_rate(*rate)));
        }
    }

    // Failed requests
    if let Some(metric) = summary.metrics.get("http_req_failed") {
        if let Some(fails) = metric.values.get("fails") {
            let rate = metric.values.get("rate").copied().unwrap_or(0.0);
            output.push_str(&format!(
                "| Failed Requests | {} ({}) |\n",
                format_count(*fails),
                format_percent(rate)
            ));
        }
    }

    // Response times
    if let Some(metric) = summary.metrics.get("http_req_duration") {
        if let Some(avg) = metric.values.get("avg") {
            output.push_str(&format!("| Avg Response Time | {} |\n", format_duration(*avg)));
        }
        if let Some(p95) = metric.values.get("p(95)") {
            output.push_str(&format!("| P95 Response Time | {} |\n", format_duration(*p95)));
        }
    }

    // Iterations
    if let Some(metric) = summary.metrics.get("iterations") {
        if let Some(count) = metric.values.get("count") {
            output.push_str(&format!("| Iterations | {} |\n", format_count(*count)));
        }
    }

    // VUs
    if let Some(metric) = summary.metrics.get("vus") {
        if let Some(value) = metric.values.get("value") {
            output.push_str(&format!("| Virtual Users | {} |\n", *value as u64));
        }
    }

    output.push_str("\n---\n\n");
    output
}

fn generate_thresholds_section(summary: &K6Summary) -> String {
    let mut thresholds: Vec<(String, String, bool)> = Vec::new();

    for (metric_name, metric) in &summary.metrics {
        for (threshold_expr, result) in &metric.thresholds {
            thresholds.push((metric_name.clone(), threshold_expr.clone(), result.ok));
        }
    }

    if thresholds.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("## Thresholds\n\n");
    output.push_str("| Metric | Threshold | Status |\n");
    output.push_str("|--------|-----------|--------|\n");

    // Sort: failed first, then by metric name
    thresholds.sort_by(|a, b| {
        if a.2 != b.2 {
            a.2.cmp(&b.2) // false (failed) first
        } else {
            a.0.cmp(&b.0)
        }
    });

    for (metric_name, threshold_expr, ok) in &thresholds {
        let status = if *ok { "PASS" } else { "**FAIL**" };
        let icon = if *ok { "✓" } else { "✗" };
        output.push_str(&format!(
            "| {} | `{}` | {} {} |\n",
            metric_name, threshold_expr, icon, status
        ));
    }

    output.push_str("\n---\n\n");
    output
}

fn generate_http_metrics_section(summary: &K6Summary) -> String {
    let http_metrics: Vec<(&String, &Metric)> = summary
        .metrics
        .iter()
        .filter(|(name, _)| name.starts_with("http_") && !name.contains('{'))
        .collect();

    if http_metrics.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("## HTTP Metrics\n\n");

    // Sort metrics by name
    let mut sorted_metrics: Vec<_> = http_metrics;
    sorted_metrics.sort_by(|a, b| a.0.cmp(b.0));

    for (name, metric) in sorted_metrics {
        output.push_str(&format!(
            "### {} ({})\n\n",
            name,
            format!("{:?}", metric.metric_type).to_lowercase()
        ));
        output.push_str("| Stat | Value |\n");
        output.push_str("|------|-------|\n");

        // Sort values by key, but put common stats first
        let priority_keys = ["avg", "min", "med", "max", "p(90)", "p(95)", "p(99)"];
        let mut sorted_values: Vec<(&String, &f64)> = metric.values.iter().collect();
        sorted_values.sort_by(|a, b| {
            let a_idx = priority_keys.iter().position(|&k| k == a.0.as_str());
            let b_idx = priority_keys.iter().position(|&k| k == b.0.as_str());
            match (a_idx, b_idx) {
                (Some(ai), Some(bi)) => ai.cmp(&bi),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.0.cmp(b.0),
            }
        });

        for (key, value) in sorted_values {
            output.push_str(&format!(
                "| {} | {} |\n",
                key,
                format_value(*value, key, &metric.contains, metric.metric_type)
            ));
        }
        output.push_str("\n");
    }

    output.push_str("---\n\n");
    output
}

fn generate_checks_section(summary: &K6Summary) -> String {
    let checks = match &summary.root_group {
        Some(group) => collect_checks(group),
        None => Vec::new(),
    };

    if checks.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    output.push_str("## Checks\n\n");
    output.push_str("| Check | Passes | Fails | Success Rate |\n");
    output.push_str("|-------|--------|-------|-------------|\n");

    for check in checks {
        let total = check.passes + check.fails;
        let rate = if total > 0 {
            (check.passes as f64 / total as f64) * 100.0
        } else {
            100.0
        };
        let status_icon = if check.fails == 0 { "✓" } else { "✗" };
        output.push_str(&format!(
            "| {} {} | {} | {} | {:.2}% |\n",
            status_icon, check.name, check.passes, check.fails, rate
        ));
    }

    output.push_str("\n---\n\n");
    output
}

fn collect_checks(group: &Group) -> Vec<&Check> {
    let mut checks: Vec<&Check> = group.checks.iter().collect();
    for subgroup in &group.groups {
        checks.extend(collect_checks(subgroup));
    }
    checks
}

fn generate_all_metrics_section(summary: &K6Summary) -> String {
    let mut output = String::new();
    output.push_str("## All Metrics\n\n");

    // Group metrics by type
    let mut counters: Vec<(&String, &Metric)> = Vec::new();
    let mut rates: Vec<(&String, &Metric)> = Vec::new();
    let mut gauges: Vec<(&String, &Metric)> = Vec::new();
    let mut trends: Vec<(&String, &Metric)> = Vec::new();

    for (name, metric) in &summary.metrics {
        // Skip sub-metrics and HTTP metrics (already shown)
        if name.contains('{') || name.starts_with("http_") {
            continue;
        }
        match metric.metric_type {
            MetricType::Counter => counters.push((name, metric)),
            MetricType::Rate => rates.push((name, metric)),
            MetricType::Gauge => gauges.push((name, metric)),
            MetricType::Trend => trends.push((name, metric)),
        }
    }

    // Counters
    if !counters.is_empty() {
        output.push_str("### Counters\n\n");
        output.push_str("| Metric | Count | Rate |\n");
        output.push_str("|--------|-------|------|\n");
        counters.sort_by(|a, b| a.0.cmp(b.0));
        for (name, metric) in &counters {
            let count = metric.values.get("count").copied().unwrap_or(0.0);
            let rate = metric.values.get("rate").copied().unwrap_or(0.0);
            output.push_str(&format!(
                "| {} | {} | {} |\n",
                name,
                format_count(count),
                format_rate(rate)
            ));
        }
        output.push_str("\n");
    }

    // Rates
    if !rates.is_empty() {
        output.push_str("### Rates\n\n");
        output.push_str("| Metric | Rate | Passes | Fails |\n");
        output.push_str("|--------|------|--------|-------|\n");
        rates.sort_by(|a, b| a.0.cmp(b.0));
        for (name, metric) in &rates {
            let rate = metric.values.get("rate").copied().unwrap_or(0.0);
            let passes = metric.values.get("passes").copied().unwrap_or(0.0);
            let fails = metric.values.get("fails").copied().unwrap_or(0.0);
            output.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                name,
                format_percent(rate),
                format_count(passes),
                format_count(fails)
            ));
        }
        output.push_str("\n");
    }

    // Gauges
    if !gauges.is_empty() {
        output.push_str("### Gauges\n\n");
        output.push_str("| Metric | Value | Min | Max |\n");
        output.push_str("|--------|-------|-----|-----|\n");
        gauges.sort_by(|a, b| a.0.cmp(b.0));
        for (name, metric) in &gauges {
            let value = metric.values.get("value").copied().unwrap_or(0.0);
            let min = metric.values.get("min").copied().unwrap_or(0.0);
            let max = metric.values.get("max").copied().unwrap_or(0.0);
            output.push_str(&format!(
                "| {} | {:.2} | {:.2} | {:.2} |\n",
                name, value, min, max
            ));
        }
        output.push_str("\n");
    }

    // Trends (non-HTTP)
    if !trends.is_empty() {
        output.push_str("### Trends\n\n");
        for (name, metric) in &trends {
            output.push_str(&format!("**{}**\n\n", name));
            output.push_str("| Stat | Value |\n");
            output.push_str("|------|-------|\n");

            let priority_keys = ["avg", "min", "med", "max", "p(90)", "p(95)", "p(99)"];
            let mut sorted_values: Vec<(&String, &f64)> = metric.values.iter().collect();
            sorted_values.sort_by(|a, b| {
                let a_idx = priority_keys.iter().position(|&k| k == a.0.as_str());
                let b_idx = priority_keys.iter().position(|&k| k == b.0.as_str());
                match (a_idx, b_idx) {
                    (Some(ai), Some(bi)) => ai.cmp(&bi),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => a.0.cmp(b.0),
                }
            });

            for (key, value) in sorted_values {
                output.push_str(&format!(
                    "| {} | {} |\n",
                    key,
                    format_value(*value, key, &metric.contains, metric.metric_type)
                ));
            }
            output.push_str("\n");
        }
    }

    output
}

// =============================================================================
// Main
// =============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Determine output path
    let output_path = cli.output.unwrap_or_else(|| cli.input.with_extension("md"));

    // Read JSON file
    let json_content = std::fs::read_to_string(&cli.input)
        .map_err(|e| format!("Failed to read '{}': {}", cli.input.display(), e))?;

    // Parse JSON
    let summary: K6Summary = serde_json::from_str(&json_content)
        .map_err(|e| format!("Failed to parse JSON: {}", e))?;

    // Generate Markdown report
    let markdown = generate_report(&summary);

    // Write output
    std::fs::write(&output_path, &markdown)
        .map_err(|e| format!("Failed to write '{}': {}", output_path.display(), e))?;

    eprintln!("Report generated: {}", output_path.display());
    Ok(())
}
