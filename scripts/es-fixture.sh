#!/usr/bin/env bash
#
# The Elasticsearch fixture for the integration tests: one node in Docker, on
# the Basic license or a trial license.
#
#   scripts/es-fixture.sh up [basic|trial]   start it and wait until it's ready
#   scripts/es-fixture.sh down               stop it and delete its data
#   scripts/es-fixture.sh logs               print its logs
#
# The version is set in integration/compose.yaml, or by ELASTICSEARCH_VERSION.
# It listens on 127.0.0.1:${OXSEARCH_TEST_PORT:-9200}. The tests read
# OXSEARCH_TEST_URL and OXSEARCH_TEST_LICENSE; `up` prints both.

set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
compose=(docker compose -f "${root}/integration/compose.yaml")
url="http://127.0.0.1:${OXSEARCH_TEST_PORT:-9200}"

license_type() {
  curl -fsS "${url}/_license" | sed -n 's/.*"type" *: *"\([a-z]*\)".*/\1/p' | head -1
}

up() {
  license="${1:-basic}"
  case "$license" in
    basic | trial) ;;
    *) echo "unknown license: ${license} (basic or trial)" >&2; exit 2 ;;
  esac

  "${compose[@]}" up -d --wait --wait-timeout 300

  # A new node installs its Basic license a moment after the cluster turns
  # yellow; until then, GET /_license is a 404.
  for _ in $(seq 1 60); do
    curl -fsS -o /dev/null "${url}/_license" 2>/dev/null && break
    sleep 1
  done

  if [ "$license" = trial ]; then
    # The same call a demo-day trial uses. A node can start one trial in its
    # life, so a second `up trial` on a running fixture finds it already on.
    if [ "$(license_type)" != trial ]; then
      curl -fsS -X POST "${url}/_license/start_trial?acknowledge=true" >/dev/null
    fi
  fi

  actual="$(license_type)"
  if [ "$actual" != "$license" ]; then
    echo "the fixture runs the ${actual:-unknown} license, not ${license}" >&2
    echo "(a running fixture can't go back from trial to basic: run 'down' first)" >&2
    exit 1
  fi

  version="$(curl -fsS "${url}/" | sed -n 's/.*"number" *: *"\([^"]*\)".*/\1/p' | head -1)"
  echo "Elasticsearch ${version} on the ${license} license at ${url}"
  echo "export OXSEARCH_TEST_URL=${url} OXSEARCH_TEST_LICENSE=${license}"
}

case "${1:-}" in
  up) up "${2:-basic}" ;;
  down) "${compose[@]}" down --volumes ;;
  logs) "${compose[@]}" logs --no-color ;;
  *) echo "usage: scripts/es-fixture.sh up [basic|trial] | down | logs" >&2; exit 2 ;;
esac
