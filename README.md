# k6r

A CLI tool to convert K6 handleSummary JSON to Markdown reports.

## Installation

```bash
cargo install --path .
```

Or use the release binary directly:

```bash
cargo build --release
./target/release/k6r
```

## Usage

```bash
# Basic usage (creates .md file with same name)
k6r summary.json

# Specify output file
k6r summary.json report.md

# Help
k6r --help
```

## Supported Formats

k6r automatically detects the input format:

### 1. handleSummary JSON (recommended)

Add this to your K6 script:

```javascript
export function handleSummary(data) {
  return {
    'summary.json': JSON.stringify(data),
  };
}
```

### 2. JSONL format (`--out json`)

No script modification needed:

```bash
k6 run --out json=results.json script.js
```

**Note:** JSONL format requires k6r to calculate statistics from raw data points. Checks information is not available in this format.

## Generated Report Sections

- **Summary**: Total requests, failure rate, avg/P95 response times
- **Thresholds**: Pass/fail status for defined thresholds
- **HTTP Metrics**: Detailed breakdown of http_req_duration, etc.
- **Checks**: Success/failure statistics for each check
- **All Metrics**: Counters, Rates, Gauges, and Trends

## Example Output

```markdown
# K6 Load Test Report

**Test Duration:** 30.00s

---

## Summary

| Metric | Value |
|--------|-------|
| Total Requests | 1000 |
| Request Rate | 33.33/s |
| Failed Requests | 20 (2.00%) |
| Avg Response Time | 150.25ms |
| P95 Response Time | 450.00ms |

## Thresholds

| Metric | Threshold | Status |
|--------|-----------|--------|
| http_req_duration | `p(95)<500` | ✓ PASS |
| http_req_failed | `rate<0.1` | ✓ PASS |
```

## Disclaimer

This project was generated with the assistance of AI tools (Claude). The author is not proficient in Rust, so the code quality may not be optimal. Use at your own discretion and feel free to contribute improvements.

## License

MIT
