#!/usr/bin/env bash
# =============================================================================
# test/otel-e2e/run.sh  —  OTEL E2E harness for the Buzz relay
# =============================================================================
#
# Proves both export surfaces introduced in PR #1398:
#
#   (a) Prometheus /metrics contains expected buzz_* series with non-zero
#       values and does NOT contain a target_info series.
#   (b) OTLP traces: the collector received ws.auth and ws.event spans
#       carrying a conn_id attribute tagged service.name=buzz-relay.
#   (c) OTLP metrics: the collector received data tagged service.name=buzz-relay.
#   (d) OTLP-disabled control: with OTEL_EXPORTER_OTLP_ENDPOINT unset the
#       relay still serves /metrics; the collector receives nothing.
#
# Usage:
#   just otel-e2e        # canonical one-command entry point
#   ./test/otel-e2e/run.sh [--skip-build]
#
# Prerequisites:
#   - Docker (for compose services + collector)
#   - Rust toolchain (for relay + test binary)
#   - psql on PATH (or docker exec fallback if not found)
#   - Ports 3000, 4317, 5432, 6379, 9000, 9102 available
#
# The relay runs on the HOST (matching the existing host.docker.internal
# Prometheus pattern).  Only postgres, redis, minio, and the otel-collector
# run in Docker.
#
# RELAY_BINARY override:
#   By default this script builds the relay from REPO_ROOT.  To test a relay
#   built from a different branch (e.g. the otel-migration PR), set:
#
#     RELAY_BINARY=/path/to/buzz-relay just otel-e2e --skip-build
#
#   or just point at the otel-migration worktree's ci binary:
#
#     RELAY_BINARY=$(pwd)/../buzz/.worktrees/duncan-otel-migration/target/ci/buzz-relay \
#       just otel-e2e --skip-build
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

SKIP_BUILD="${1:-}"

# ── Colors ────────────────────────────────────────────────────────────────────
BLUE='\033[0;34m'
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

log()  { echo -e "${BLUE}[otel-e2e]${NC} $*"; }
ok()   { echo -e "${GREEN}[otel-e2e ✓]${NC} $*"; }
warn() { echo -e "${YELLOW}[otel-e2e !]${NC} $*"; }
err()  { echo -e "${RED}[otel-e2e ✗]${NC} $*" >&2; }

# ── Relay PID tracking ────────────────────────────────────────────────────────
RELAY_PID_FILE=/tmp/buzz-otel-e2e-relay.pid

cleanup() {
  log "Tearing down..."
  # Kill relay if running.
  if [[ -f "${RELAY_PID_FILE}" ]]; then
    RELAY_PID=$(cat "${RELAY_PID_FILE}")
    if kill -0 "${RELAY_PID}" 2>/dev/null; then
      kill "${RELAY_PID}" && wait "${RELAY_PID}" 2>/dev/null || true
    fi
    rm -f "${RELAY_PID_FILE}"
  fi
  # Bring down compose stack (including collector) and remove volumes.
  cd "${REPO_ROOT}"
  docker compose -f docker-compose.yml -f test/otel-e2e/compose.otel-e2e.yml \
    down -v --remove-orphans 2>/dev/null || true
  ok "Teardown complete"
}
trap cleanup EXIT

# ── Step 1: Start backing services + otel-collector ──────────────────────────
cd "${REPO_ROOT}"

log "Starting backing services and otel-collector..."
docker compose -f docker-compose.yml -f test/otel-e2e/compose.otel-e2e.yml \
  up -d postgres redis minio minio-init otel-collector

# ── Wait helpers ──────────────────────────────────────────────────────────────
wait_healthy() {
  local service="$1" container="$2"
  log "Waiting for ${service}..."
  for _ in $(seq 1 60); do
    status=$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || echo "not_found")
    if [[ "${status}" == "healthy" ]]; then
      ok "${service} healthy"
      return 0
    fi
    sleep 2
  done
  err "${service} did not become healthy within 120s"
  docker logs "${container}" || true
  return 1
}

wait_healthy "Postgres"        "buzz-postgres"
wait_healthy "Redis"           "buzz-redis"
wait_healthy "MinIO"           "buzz-minio"

# otel-collector: healthcheck uses nc; retry with a TCP connect fallback.
log "Waiting for otel-collector (gRPC :4317)..."
for _ in $(seq 1 30); do
  if docker inspect --format='{{.State.Health.Status}}' "buzz-otel-collector" 2>/dev/null | grep -q "healthy"; then
    ok "otel-collector healthy"
    break
  fi
  # Fallback: try TCP connect from host.
  if nc -z -w1 127.0.0.1 4317 2>/dev/null; then
    ok "otel-collector reachable on :4317"
    break
  fi
  sleep 2
done

# ── Step 2: Apply DB schema ───────────────────────────────────────────────────
log "Applying database schema..."
export PGHOST=localhost PGPORT=5432 PGUSER=buzz PGPASSWORD=buzz_dev PGDATABASE=buzz

if command -v psql >/dev/null 2>&1; then
  seed_psql() { PGPASSWORD="${PGPASSWORD}" psql -h "${PGHOST}" -p "${PGPORT}" -U "${PGUSER}" -d "${PGDATABASE}" -qtA "$@"; }
else
  seed_psql() { docker exec -e PGPASSWORD="${PGPASSWORD}" buzz-postgres psql -U "${PGUSER}" -d "${PGDATABASE}" -qtA "$@"; }
fi

"${REPO_ROOT}/bin/pgschema" apply --file "${REPO_ROOT}/schema/schema.sql" --auto-approve
docker exec -i -e PGPASSWORD="${PGPASSWORD}" buzz-postgres \
  psql -U "${PGUSER}" -d "${PGDATABASE}" -v ON_ERROR_STOP=1 \
  < "${REPO_ROOT}/scripts/attach-schema-partitions.sql"

seed_psql -c "
INSERT INTO communities (id, host)
VALUES ('00000000-0000-4000-8000-00000000c0de', 'localhost:3000')
ON CONFLICT (lower(host)) DO NOTHING;
"
ok "Schema applied and community seeded"

# ── Step 3: Build relay (unless --skip-build) ────────────────────────────────
RELAY_BIN="${RELAY_BINARY:-${REPO_ROOT}/target/ci/buzz-relay}"
if [[ "${SKIP_BUILD}" != "--skip-build" ]]; then
  log "Building relay (profile: ci)..."
  cd "${REPO_ROOT}"
  cargo build --profile ci -p buzz-relay
  RELAY_BIN="${REPO_ROOT}/target/ci/buzz-relay"
  ok "Relay built"
else
  if [[ ! -x "${RELAY_BIN}" ]]; then
    err "RELAY_BINARY=${RELAY_BIN} does not exist or is not executable"
    err "Either remove --skip-build to build, or set RELAY_BINARY to a pre-built binary"
    exit 1
  fi
  log "Using pre-built relay: ${RELAY_BIN}"
fi

# ── Step 4: Set up collector output readback ─────────────────────────────────
# The file exporter writes to /tmp/otelcol-output inside the collector container,
# which is bind-mounted to /tmp/buzz-otel-e2e-output on the host (see compose overlay).
# The collector image is distroless so we read directly from the host path.
COLLECTOR_OUTPUT_DIR=/tmp/buzz-otel-e2e-output
mkdir -p "${COLLECTOR_OUTPUT_DIR}"
COLLECTOR_OUTPUT_HOST="${COLLECTOR_OUTPUT_DIR}/telemetry.json"

# ── Step 5: Start relay WITH OTLP enabled ────────────────────────────────────
log "Starting relay (OTLP enabled → http://localhost:4317)..."
RELAY_LOG=/tmp/buzz-otel-e2e-relay-otlp.log

nohup env \
  DATABASE_URL="postgres://buzz:buzz_dev@localhost:5432/buzz" \
  REDIS_URL="redis://localhost:6379" \
  RELAY_URL="ws://localhost:3000" \
  BUZZ_BIND_ADDR="0.0.0.0:3000" \
  BUZZ_REQUIRE_AUTH_TOKEN=false \
  BUZZ_RECONCILE_CHANNELS=true \
  OTEL_EXPORTER_OTLP_ENDPOINT="http://localhost:4317" \
  OTEL_SERVICE_NAME="buzz-relay" \
  RUST_LOG="buzz_relay=info" \
  "${RELAY_BIN}" > "${RELAY_LOG}" 2>&1 &
RELAY_PID=$!
echo "${RELAY_PID}" > "${RELAY_PID_FILE}"

log "Waiting for relay readiness (OTLP enabled)..."
for attempt in $(seq 1 60); do
  if ! kill -0 "${RELAY_PID}" 2>/dev/null; then
    err "Relay process died during startup"
    cat "${RELAY_LOG}"
    exit 1
  fi
  status_code=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/_readiness || true)
  if [[ "${status_code}" == "200" ]]; then
    ok "Relay ready (OTLP enabled)"
    break
  fi
  sleep 1
  if [[ "${attempt}" -eq 60 ]]; then
    err "Relay did not become ready within 60s"
    cat "${RELAY_LOG}"
    exit 1
  fi
done

# ── Step 6: Run OTLP-enabled tests (a), (b), (c) ────────────────────────────
log "Waiting for OTLP exports to flush (batch processor 1s timeout + buffer)..."
sleep 5  # Give the relay's batch exporter time to flush after readiness

COLLECTOR_LINE_COUNT=$(wc -l < "${COLLECTOR_OUTPUT_HOST}" 2>/dev/null || echo 0)
log "Collector has ${COLLECTOR_LINE_COUNT} lines so far (may be low — OTLP metrics are periodic)"

log "Running OTLP-enabled tests (a)(b)(c)..."
RELAY_URL="ws://localhost:3000" \
METRICS_URL="http://localhost:9102/metrics" \
OTEL_COLLECTOR_OUTPUT="${COLLECTOR_OUTPUT_HOST}" \
  cargo test -p buzz-test-client --test e2e_otel \
    test_prometheus_contains_buzz_metrics \
    -- --ignored 2>&1
# Note: test_prometheus_contains_buzz_metrics matches test_prometheus_contains_buzz_metrics_with_nonzero_values

# Give traces time to flush from the test's WS interaction.
sleep 5

RELAY_URL="ws://localhost:3000" \
METRICS_URL="http://localhost:9102/metrics" \
OTEL_COLLECTOR_OUTPUT="${COLLECTOR_OUTPUT_HOST}" \
  cargo test -p buzz-test-client --test e2e_otel \
    test_otlp \
    -- --ignored 2>&1
ok "OTLP-enabled assertions passed"

# ── Step 7: Stop the OTLP-enabled relay ──────────────────────────────────────
log "Stopping OTLP-enabled relay..."
kill "${RELAY_PID}" && wait "${RELAY_PID}" 2>/dev/null || true
rm -f "${RELAY_PID_FILE}"

# Save collector output for inspection.
log "Saving collector output for inspection..."
COLLECTOR_LINE_COUNT=$(wc -l < "${COLLECTOR_OUTPUT_HOST}" 2>/dev/null || echo 0)
ok "Collector received data: ${COLLECTOR_LINE_COUNT} lines at ${COLLECTOR_OUTPUT_HOST}"

# Print the span names that appeared for human inspection.
log "Span names found in collector output:"
cat "${COLLECTOR_OUTPUT_HOST}" | python3 -c "
import sys, json, re
spans = set()
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        blob = json.dumps(json.loads(line))
        spans.update(re.findall(r'\"name\":\s*\"([^\"]+)\"', blob))
    except Exception:
        pass
for s in sorted(spans):
    print(' ', s)
" || true

# ── Step 8: Clear collector output, start relay WITHOUT OTLP ─────────────────
log "Clearing collector output for disabled-path test..."
rm -f "${COLLECTOR_OUTPUT_HOST}"
sleep 1

log "Starting relay (OTLP DISABLED)..."
RELAY_LOG_DISABLED=/tmp/buzz-otel-e2e-relay-nootlp.log

nohup env \
  DATABASE_URL="postgres://buzz:buzz_dev@localhost:5432/buzz" \
  REDIS_URL="redis://localhost:6379" \
  RELAY_URL="ws://localhost:3000" \
  BUZZ_BIND_ADDR="0.0.0.0:3000" \
  BUZZ_REQUIRE_AUTH_TOKEN=false \
  BUZZ_RECONCILE_CHANNELS=true \
  RUST_LOG="buzz_relay=info" \
  "${RELAY_BIN}" > "${RELAY_LOG_DISABLED}" 2>&1 &
RELAY_PID=$!
echo "${RELAY_PID}" > "${RELAY_PID_FILE}"

log "Waiting for relay readiness (OTLP disabled)..."
for attempt in $(seq 1 60); do
  if ! kill -0 "${RELAY_PID}" 2>/dev/null; then
    err "Relay process died during startup (OTLP disabled)"
    cat "${RELAY_LOG_DISABLED}"
    exit 1
  fi
  status_code=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:3000/_readiness || true)
  if [[ "${status_code}" == "200" ]]; then
    ok "Relay ready (OTLP disabled)"
    break
  fi
  sleep 1
  if [[ "${attempt}" -eq 60 ]]; then
    err "Relay did not become ready within 60s (OTLP disabled)"
    cat "${RELAY_LOG_DISABLED}"
    exit 1
  fi
done

# ── Step 9: Run disabled-path test (d) ───────────────────────────────────────
log "Running OTLP-disabled control test (d)..."
RELAY_URL="ws://localhost:3000" \
METRICS_URL="http://localhost:9102/metrics" \
  cargo test -p buzz-test-client --test e2e_otel \
    test_prometheus_works_without_otlp_endpoint \
    -- --ignored 2>&1
ok "OTLP-disabled assertion passed"
# Stop relay.
kill "${RELAY_PID}" && wait "${RELAY_PID}" 2>/dev/null || true
rm -f "${RELAY_PID_FILE}"

# Assert collector received NOTHING during the disabled run.
log "Asserting collector received nothing during OTLP-disabled run..."
sleep 3  # Give any possible OTLP traffic time to arrive (there should be none)
DISABLED_OUTPUT=""
if [[ -f "${COLLECTOR_OUTPUT_HOST}" ]]; then
  DISABLED_OUTPUT=$(cat "${COLLECTOR_OUTPUT_HOST}" | tr -d '[:space:]')
fi
if [[ -n "${DISABLED_OUTPUT}" ]]; then
  err "OTLP-disabled assertion FAILED: collector received data when OTEL_EXPORTER_OTLP_ENDPOINT was unset"
  head -5 "${COLLECTOR_OUTPUT_HOST}"
  exit 1
fi
ok "OTLP-disabled: collector received nothing ✓"

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║  OTEL E2E HARNESS — ALL ASSERTIONS PASSED                ║${NC}"
echo -e "${GREEN}╠══════════════════════════════════════════════════════════╣${NC}"
echo -e "${GREEN}║  (a) Prometheus buzz_* series present, non-zero, no      ║${NC}"
echo -e "${GREEN}║      target_info                                          ║${NC}"
echo -e "${GREEN}║  (b) OTLP traces: ws.auth + ws.event with conn_id        ║${NC}"
echo -e "${GREEN}║  (c) OTLP metrics: service.name=buzz-relay present       ║${NC}"
echo -e "${GREEN}║  (d) OTLP-disabled: /metrics works, collector silent     ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"
