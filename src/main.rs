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
#[command(about = "Convert K6 JSON output to Markdown reports")]
struct Cli {
    /// Input K6 JSON file (handleSummary or --out json format)
    #[arg(value_name = "JSON_FILE")]
    input: PathBuf,

    /// Output Markdown file (defaults to input filename with .md extension)
    #[arg(value_name = "MARKDOWN_FILE")]
    output: Option<PathBuf>,
}

// =============================================================================
// Data Model - handleSummary format
// =============================================================================

#[derive(Debug, Deserialize, Default)]
pub struct K6Summary {
    #[serde(default)]
    pub metrics: HashMap<String, Metric>,
    pub root_group: Option<Group>,
    pub state: Option<State>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Metric {
    #[serde(rename = "type")]
    pub metric_type: MetricType,
    #[serde(default)]
    pub contains: String,
    #[serde(default)]
    pub values: HashMap<String, f64>,
    #[serde(default)]
    pub thresholds: HashMap<String, Threshold>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MetricType {
    Counter,
    Rate,
    Gauge,
    #[default]
    Trend,
}

#[derive(Debug, Deserialize, Clone)]
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
// Data Model - JSONL format (--out json)
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct JsonlLine {
    #[serde(rename = "type")]
    pub line_type: String,
    pub metric: String,
    pub data: JsonlData,
}

#[derive(Debug, Deserialize)]
pub struct JsonlData {
    // For Metric type
    #[serde(rename = "type")]
    pub metric_type: Option<String>,
    pub contains: Option<String>,
    #[serde(default)]
    pub thresholds: Vec<String>,

    // For Point type
    pub time: Option<String>,
    pub value: Option<f64>,
    #[serde(default)]
    pub tags: Option<HashMap<String, serde_json::Value>>,
}

// =============================================================================
// JSONL Parser
// =============================================================================

struct MetricCollector {
    metric_type: MetricType,
    contains: String,
    values: Vec<f64>,
    thresholds: Vec<String>,
}

fn parse_jsonl(content: &str) -> K6Summary {
    let mut collectors: HashMap<String, MetricCollector> = HashMap::new();
    let mut first_time: Option<String> = None;
    let mut last_time: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parsed: Result<JsonlLine, _> = serde_json::from_str(line);
        let Ok(entry) = parsed else { continue };

        match entry.line_type.as_str() {
            "Metric" => {
                let metric_type = match entry.data.metric_type.as_deref() {
                    Some("counter") => MetricType::Counter,
                    Some("rate") => MetricType::Rate,
                    Some("gauge") => MetricType::Gauge,
                    Some("trend") => MetricType::Trend,
                    _ => MetricType::Trend,
                };

                collectors.entry(entry.metric.clone()).or_insert(MetricCollector {
                    metric_type,
                    contains: entry.data.contains.unwrap_or_default(),
                    values: Vec::new(),
                    thresholds: entry.data.thresholds,
                });
            }
            "Point" => {
                if let Some(value) = entry.data.value {
                    // Track time range
                    if let Some(time) = &entry.data.time {
                        if first_time.is_none() {
                            first_time = Some(time.clone());
                        }
                        last_time = Some(time.clone());
                    }

                    // Skip sub-metrics (with tags like {expected_response:true})
                    if entry.data.tags.as_ref().map_or(false, |t| !t.is_empty()) {
                        // Check if it has meaningful tags (not just "group")
                        if let Some(tags) = &entry.data.tags {
                            let dominated_keys: Vec<_> = tags.keys().filter(|k| *k != "group").collect();
                            if !dominated_keys.is_empty() {
                                continue;
                            }
                        }
                    }

                    collectors
                        .entry(entry.metric.clone())
                        .or_insert(MetricCollector {
                            metric_type: MetricType::Trend,
                            contains: String::new(),
                            values: Vec::new(),
                            thresholds: Vec::new(),
                        })
                        .values
                        .push(value);
                }
            }
            _ => {}
        }
    }

    // Calculate duration from timestamps
    let duration_ms = calculate_duration(&first_time, &last_time);

    // Convert collectors to metrics
    let mut metrics: HashMap<String, Metric> = HashMap::new();

    for (name, collector) in collectors {
        let values = calculate_stats(&collector.values, collector.metric_type);

        let thresholds: HashMap<String, Threshold> = collector
            .thresholds
            .iter()
            .map(|t| (t.clone(), Threshold { ok: true })) // Can't determine pass/fail from JSONL
            .collect();

        metrics.insert(
            name,
            Metric {
                metric_type: collector.metric_type,
                contains: collector.contains,
                values,
                thresholds,
            },
        );
    }

    K6Summary {
        metrics,
        root_group: None,
        state: duration_ms.map(|ms| State {
            test_run_duration_ms: ms,
        }),
    }
}

fn calculate_duration(first: &Option<String>, last: &Option<String>) -> Option<f64> {
    let first = first.as_ref()?;
    let last = last.as_ref()?;

    // Parse ISO 8601 timestamps
    let parse_time = |s: &str| -> Option<f64> {
        // Simple parsing: extract seconds and milliseconds
        // Format: 2017-05-09T14:34:45.625742514+02:00
        let parts: Vec<&str> = s.split('T').collect();
        if parts.len() != 2 {
            return None;
        }
        let time_part = parts[1].split('+').next()?.split('-').next()?;
        let time_components: Vec<&str> = time_part.split(':').collect();
        if time_components.len() != 3 {
            return None;
        }
        let hours: f64 = time_components[0].parse().ok()?;
        let minutes: f64 = time_components[1].parse().ok()?;
        let seconds: f64 = time_components[2].parse().ok()?;
        Some((hours * 3600.0 + minutes * 60.0 + seconds) * 1000.0)
    };

    let first_ms = parse_time(first)?;
    let last_ms = parse_time(last)?;

    Some((last_ms - first_ms).abs())
}

fn calculate_stats(values: &[f64], metric_type: MetricType) -> HashMap<String, f64> {
    let mut stats = HashMap::new();

    if values.is_empty() {
        return stats;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let sum: f64 = values.iter().sum();
    let count = values.len() as f64;

    match metric_type {
        MetricType::Counter => {
            stats.insert("count".to_string(), count);
            stats.insert("rate".to_string(), count / (sum / 1000.0).max(1.0)); // rough estimate
        }
        MetricType::Rate => {
            let passes = values.iter().filter(|&&v| v > 0.0).count() as f64;
            let fails = count - passes;
            stats.insert("rate".to_string(), passes / count);
            stats.insert("passes".to_string(), passes);
            stats.insert("fails".to_string(), fails);
        }
        MetricType::Gauge => {
            stats.insert("value".to_string(), *sorted.last().unwrap_or(&0.0));
            stats.insert("min".to_string(), *sorted.first().unwrap_or(&0.0));
            stats.insert("max".to_string(), *sorted.last().unwrap_or(&0.0));
        }
        MetricType::Trend => {
            stats.insert("avg".to_string(), sum / count);
            stats.insert("min".to_string(), *sorted.first().unwrap_or(&0.0));
            stats.insert("max".to_string(), *sorted.last().unwrap_or(&0.0));
            stats.insert("med".to_string(), percentile(&sorted, 50.0));
            stats.insert("p(90)".to_string(), percentile(&sorted, 90.0));
            stats.insert("p(95)".to_string(), percentile(&sorted, 95.0));
            stats.insert("p(99)".to_string(), percentile(&sorted, 99.0));
        }
    }

    stats
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let index = (p / 100.0) * (sorted.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    let fraction = index - lower as f64;

    if upper >= sorted.len() {
        sorted[sorted.len() - 1]
    } else {
        sorted[lower] + fraction * (sorted[upper] - sorted[lower])
    }
}

// =============================================================================
// Format Detection
// =============================================================================

enum FileFormat {
    HandleSummary,
    Jsonl,
}

fn detect_format(content: &str) -> FileFormat {
    let trimmed = content.trim();

    // handleSummary format starts with { and contains "metrics" key
    if trimmed.starts_with('{') {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            if value.get("metrics").is_some() {
                return FileFormat::HandleSummary;
            }
        }
    }

    // Otherwise assume JSONL
    FileFormat::Jsonl
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

    output.push_str("# K6 Load Test Report\n\n");

    if let Some(state) = &summary.state {
        output.push_str(&format!(
            "**Test Duration:** {}\n\n",
            format_duration(state.test_run_duration_ms)
        ));
    }

    output.push_str("---\n\n");
    output.push_str(&generate_summary_section(summary));
    output.push_str(&generate_thresholds_section(summary));
    output.push_str(&generate_http_metrics_section(summary));
    output.push_str(&generate_checks_section(summary));
    output.push_str(&generate_all_metrics_section(summary));

    output
}

fn generate_summary_section(summary: &K6Summary) -> String {
    let mut output = String::new();
    output.push_str("## Summary\n\n");
    output.push_str("| Metric | Value |\n");
    output.push_str("|--------|-------|\n");

    if let Some(metric) = summary.metrics.get("http_reqs") {
        if let Some(count) = metric.values.get("count") {
            output.push_str(&format!("| Total Requests | {} |\n", format_count(*count)));
        }
        if let Some(rate) = metric.values.get("rate") {
            output.push_str(&format!("| Request Rate | {} |\n", format_rate(*rate)));
        }
    }

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

    if let Some(metric) = summary.metrics.get("http_req_duration") {
        if let Some(avg) = metric.values.get("avg") {
            output.push_str(&format!("| Avg Response Time | {} |\n", format_duration(*avg)));
        }
        if let Some(p95) = metric.values.get("p(95)") {
            output.push_str(&format!("| P95 Response Time | {} |\n", format_duration(*p95)));
        }
    }

    if let Some(metric) = summary.metrics.get("iterations") {
        if let Some(count) = metric.values.get("count") {
            output.push_str(&format!("| Iterations | {} |\n", format_count(*count)));
        }
    }

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

    thresholds.sort_by(|a, b| {
        if a.2 != b.2 {
            a.2.cmp(&b.2)
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

    let mut counters: Vec<(&String, &Metric)> = Vec::new();
    let mut rates: Vec<(&String, &Metric)> = Vec::new();
    let mut gauges: Vec<(&String, &Metric)> = Vec::new();
    let mut trends: Vec<(&String, &Metric)> = Vec::new();

    for (name, metric) in &summary.metrics {
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

    if !trends.is_empty() {
        output.push_str("### Trends\n\n");
        trends.sort_by(|a, b| a.0.cmp(b.0));
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

    let output_path = cli.output.unwrap_or_else(|| cli.input.with_extension("md"));

    let content = std::fs::read_to_string(&cli.input)
        .map_err(|e| format!("Failed to read '{}': {}", cli.input.display(), e))?;

    let summary = match detect_format(&content) {
        FileFormat::HandleSummary => {
            eprintln!("Detected format: handleSummary JSON");
            serde_json::from_str(&content)
                .map_err(|e| format!("Failed to parse JSON: {}", e))?
        }
        FileFormat::Jsonl => {
            eprintln!("Detected format: JSONL (--out json)");
            parse_jsonl(&content)
        }
    };

    let markdown = generate_report(&summary);

    std::fs::write(&output_path, &markdown)
        .map_err(|e| format!("Failed to write '{}': {}", output_path.display(), e))?;

    eprintln!("Report generated: {}", output_path.display());
    Ok(())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0.5), "500.00µs");
        assert_eq!(format_duration(1.0), "1.00ms");
        assert_eq!(format_duration(150.5), "150.50ms");
        assert_eq!(format_duration(1500.0), "1.50s");
        assert_eq!(format_duration(90000.0), "1.50m");
    }

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(50.0), "50");
        assert_eq!(format_count(1500.0), "1.50K");
        assert_eq!(format_count(2500000.0), "2.50M");
    }

    #[test]
    fn test_format_percent() {
        assert_eq!(format_percent(0.0), "0.00%");
        assert_eq!(format_percent(0.5), "50.00%");
        assert_eq!(format_percent(1.0), "100.00%");
    }

    #[test]
    fn test_percentile() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&values, 50.0), 5.5);
        assert_eq!(percentile(&values, 100.0), 10.0);
    }

    #[test]
    fn test_percentile_empty() {
        let values: Vec<f64> = vec![];
        assert_eq!(percentile(&values, 50.0), 0.0);
    }

    #[test]
    fn test_percentile_single() {
        let values = vec![42.0];
        assert_eq!(percentile(&values, 50.0), 42.0);
        assert_eq!(percentile(&values, 95.0), 42.0);
    }

    #[test]
    fn test_calculate_stats_trend() {
        let values = vec![100.0, 200.0, 300.0, 400.0, 500.0];
        let stats = calculate_stats(&values, MetricType::Trend);

        assert_eq!(stats.get("avg"), Some(&300.0));
        assert_eq!(stats.get("min"), Some(&100.0));
        assert_eq!(stats.get("max"), Some(&500.0));
        assert_eq!(stats.get("med"), Some(&300.0));
    }

    #[test]
    fn test_calculate_stats_counter() {
        let values = vec![1.0, 1.0, 1.0, 1.0, 1.0];
        let stats = calculate_stats(&values, MetricType::Counter);

        assert_eq!(stats.get("count"), Some(&5.0));
        assert!(stats.get("rate").is_some());
    }

    #[test]
    fn test_calculate_stats_rate() {
        let values = vec![1.0, 1.0, 1.0, 0.0, 0.0]; // 3 passes, 2 fails
        let stats = calculate_stats(&values, MetricType::Rate);

        assert_eq!(stats.get("passes"), Some(&3.0));
        assert_eq!(stats.get("fails"), Some(&2.0));
        assert_eq!(stats.get("rate"), Some(&0.6)); // 3/5 = 0.6
    }

    #[test]
    fn test_detect_format_handle_summary() {
        let content = r#"{"metrics":{"http_reqs":{"type":"counter"}}}"#;
        assert!(matches!(detect_format(content), FileFormat::HandleSummary));
    }

    #[test]
    fn test_detect_format_jsonl() {
        let content = r#"{"type":"Metric","metric":"http_reqs","data":{}}
{"type":"Point","metric":"http_reqs","data":{"value":1}}"#;
        assert!(matches!(detect_format(content), FileFormat::Jsonl));
    }

    #[test]
    fn test_parse_jsonl_basic() {
        let content = r#"{"type":"Metric","data":{"type":"trend","contains":"time","thresholds":[]},"metric":"http_req_duration"}
{"type":"Point","data":{"time":"2024-01-01T10:00:00.000+00:00","value":100.0,"tags":null},"metric":"http_req_duration"}
{"type":"Point","data":{"time":"2024-01-01T10:00:01.000+00:00","value":200.0,"tags":null},"metric":"http_req_duration"}"#;

        let summary = parse_jsonl(content);

        assert!(summary.metrics.contains_key("http_req_duration"));
        let metric = summary.metrics.get("http_req_duration").unwrap();
        assert_eq!(metric.metric_type, MetricType::Trend);
        assert_eq!(metric.values.get("avg"), Some(&150.0));
        assert_eq!(metric.values.get("min"), Some(&100.0));
        assert_eq!(metric.values.get("max"), Some(&200.0));
    }

    #[test]
    fn test_parse_handle_summary() {
        let content = r#"{
            "metrics": {
                "http_reqs": {
                    "type": "counter",
                    "contains": "default",
                    "values": {"count": 100, "rate": 10.0},
                    "thresholds": {}
                }
            }
        }"#;

        let summary: K6Summary = serde_json::from_str(content).unwrap();

        assert!(summary.metrics.contains_key("http_reqs"));
        let metric = summary.metrics.get("http_reqs").unwrap();
        assert_eq!(metric.metric_type, MetricType::Counter);
        assert_eq!(metric.values.get("count"), Some(&100.0));
    }

    #[test]
    fn test_generate_report_not_empty() {
        let mut metrics = HashMap::new();
        metrics.insert(
            "http_reqs".to_string(),
            Metric {
                metric_type: MetricType::Counter,
                contains: "default".to_string(),
                values: [("count".to_string(), 100.0), ("rate".to_string(), 10.0)]
                    .into_iter()
                    .collect(),
                thresholds: HashMap::new(),
            },
        );

        let summary = K6Summary {
            metrics,
            root_group: None,
            state: Some(State {
                test_run_duration_ms: 10000.0,
            }),
        };

        let report = generate_report(&summary);

        assert!(report.contains("# K6 Load Test Report"));
        assert!(report.contains("10.00s"));
        assert!(report.contains("100"));
    }
}
