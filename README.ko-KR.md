# k6r

K6 handleSummary JSON을 Markdown 리포트로 변환하는 CLI 도구

## 설치

```bash
cargo install --path .
```

또는 릴리즈 바이너리 직접 사용:

```bash
cargo build --release
./target/release/k6r
```

## 사용법

```bash
# 기본 사용 (같은 이름의 .md 파일 생성)
k6r summary.json

# 출력 파일 지정
k6r summary.json report.md

# 도움말
k6r --help
```

## K6 스크립트에서 JSON 출력

```javascript
export function handleSummary(data) {
  return {
    'summary.json': JSON.stringify(data),
  };
}
```

## 생성되는 리포트

- **Summary**: 총 요청, 실패율, 평균/P95 응답 시간
- **Thresholds**: 임계값 통과/실패 결과
- **HTTP Metrics**: http_req_duration 등 상세 메트릭
- **Checks**: 체크 성공/실패 통계
- **All Metrics**: Counters, Rates, Gauges, Trends

## 예시 출력

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

## 면책 조항

이 프로젝트는 AI 도구(Claude)의 도움으로 생성되었습니다. 작성자는 Rust에 능숙하지 않으므로 코드 품질이 최적이 아닐 수 있습니다. 재량에 따라 사용하시고, 개선 사항이 있으면 기여해 주세요.

## 라이선스

MIT
