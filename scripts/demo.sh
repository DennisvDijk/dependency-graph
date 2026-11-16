#!/usr/bin/env bash
# Run the whole depgraph flow on the bundled sample data. No token, no network.
#
#   scripts/demo.sh            # uses `depgraph` on PATH, else builds with cargo
#   DEPGRAPH=./target/release/depgraph scripts/demo.sh
#
# Work happens in a temporary directory; the repository is not modified.
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
fixtures="$repo/tests/fixtures"

if [ -n "${DEPGRAPH:-}" ]; then
  bin="$DEPGRAPH"
elif command -v depgraph >/dev/null 2>&1; then
  bin="$(command -v depgraph)"
else
  echo "building depgraph (first run takes a minute)..."
  cargo build --release --quiet --manifest-path "$repo/Cargo.toml"
  bin="$repo/target/release/depgraph"
fi

work="${DEPGRAPH_DEMO_DIR:-$(mktemp -d)}"
mkdir -p "$work/config"
cd "$work"
command cp -f "$repo/config/"*.yaml config/
command cp -f "$fixtures/phase1/aliases.yaml" config/aliases.yaml

# Map the sample namespaces onto dev, acc, prod. Without this, Istio calls have no environment.
python3 - <<'PY'
import re
p = "config/environments.yaml"
s = open(p).read()
for env in ("dev", "acc", "prod"):
    s = re.sub(rf"(^{env}:\n(?:.*\n)*?)  k8s_namespaces: \[\]", rf"\1  k8s_namespaces:\n    - payments-{env}", s, count=1, flags=re.M)
open(p, "w").write(s)
PY

step() { printf '\n\033[1m== %s\033[0m\n$ %s\n' "$1" "$2"; }

step "1. Collect (sample repos + recorded Istio metrics)" "depgraph collect --set full --fixture ... --prometheus-fixture ..."
"$bin" collect --set full --fixture "$fixtures/phase1" --prometheus-fixture "$fixtures/phase2" --output data

step "2. Build the graph" "depgraph build"
"$bin" build --raw data/raw --output data/snapshot.json

step "3. What does payments-api depend on in prod?" "depgraph deps payments-api --env prod"
"$bin" deps payments-api --env prod --snapshot data/snapshot.json

step "4. Who breaks if billing-api breaks?" "depgraph impact billing-api --env prod"
"$bin" impact billing-api --env prod --snapshot data/snapshot.json

step "5. Declared versus observed (drift)" "depgraph drift --env prod"
"$bin" drift --env prod --snapshot data/snapshot.json --output drift_report >/dev/null
cat drift_report/prod.md

step "6. Deploy order" "depgraph deploy-order"
"$bin" deploy-order --snapshot data/snapshot.json

step "7. What would the MR check say about an API change in billing-api?" "depgraph mr-check --dry-run ..."
"$bin" mr-check --snapshot data/snapshot.json --project example-group/pilot/billing-api \
  --mr-iid 1 --changed-file openapi/billing.yaml --dry-run

step "8. Deployments and incidents on one timeline" "depgraph timeline --env prod ..."
"$bin" timeline --env prod --since 2026-01-01T00:00:00Z \
  --deployments "$fixtures/phase4/deployments.yaml" --incidents "$fixtures/phase4/incidents.yaml"

step "9. Static site" "depgraph site"
"$bin" site --snapshot data/snapshot.json --output public

cat <<MSG

Done. Everything is in $work
  open $work/public/index.html        # browse the graph
  cd $work && $bin serve-mcp --insecure-local --snapshot data/snapshot.json
                                      # then: depgraph mcp-config cursor|claude|vscode|opencode
MSG
