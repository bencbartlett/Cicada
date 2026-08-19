#!/usr/bin/env bash
# Carve speed (docs/15 measurement protocol, criterion 1): cold + warm
# `cicada run <pipeline> --node <node> --time`, 3 runs each, fresh
# --cache-dir per cold run (the warm run of a pair reuses its cold run's
# cache), reporting best/median of the solve wall time (`time: total`), the
# target node's own compute time, and the whole-process wall time.
#
# Usage (bash; Git Bash on Windows is fine):
#   corpus/measure/carve.sh [pipeline=corpus/wall.cic] [node=carved] [runs=3]
# Environment:
#   CICADA_BIN         the engine binary (default: $CARGO_TARGET_DIR/release/cicada[.exe];
#                      build it with `cargo build --release -p cicada-cli` — record
#                      numbers come from a RELEASE build, never debug)
#   CICADA_THREADS     passed as --threads when set
#   CICADA_MEASURE_OUT write the JSON result here too
# Prints a JSON result, then one summary line. Nonzero exit = a run failed.
set -u

pipeline="${1:-corpus/wall.cic}"
node="${2:-carved}"
runs="${3:-3}"
exe=""
case "$(uname -s 2>/dev/null)" in MINGW*|MSYS*|CYGWIN*|Windows_NT) exe=".exe" ;; esac
target_dir="${CARGO_TARGET_DIR:-target}"
bin="${CICADA_BIN:-$target_dir/release/cicada$exe}"
if [ ! -x "$bin" ]; then
  echo "error: no engine binary at $bin — build it: cargo build --release -p cicada-cli (or set CICADA_BIN)" >&2
  exit 2
fi
if [ ! -f "$pipeline" ]; then
  echo "error: no pipeline at $pipeline" >&2
  exit 2
fi
threads_args=()
if [ -n "${CICADA_THREADS:-}" ]; then threads_args=(--threads "$CICADA_THREADS"); fi

scratch="$(mktemp -d 2>/dev/null || mktemp -d -t cicada-carve)"
trap 'rm -rf "$scratch"' EXIT

now_ms() { date +%s%3N 2>/dev/null || python -c "import time;print(int(time.time()*1000))"; }

# one_run <cache-dir> <label> → appends a JSON object to $results
results=""
one_run() {
  local cache="$1" label="$2" t0 t1 out status solve node_ms computed hits
  t0=$(now_ms)
  out="$("$bin" run "$pipeline" --node "$node" --time --cache-dir "$cache" "${threads_args[@]}" 2>&1)"
  status=$?
  t1=$(now_ms)
  if [ $status -ne 0 ]; then
    echo "error: $label run failed (exit $status):" >&2
    echo "$out" >&2
    exit 1
  fi
  solve=$(printf '%s\n' "$out" | sed -n 's/^time: total \([0-9.]*\) ms wall.*/\1/p' | tail -1)
  computed=$(printf '%s\n' "$out" | sed -n 's/^time: total [0-9.]* ms wall — \([0-9]*\) computed.*/\1/p' | tail -1)
  hits=$(printf '%s\n' "$out" | sed -n 's/^time: total .* — [0-9]* computed, \([0-9]*\) from cache.*/\1/p' | tail -1)
  node_ms=$(printf '%s\n' "$out" | sed -n "s/^time: $node — \([0-9.]*\) ms.*/\1/p" | tail -1)
  [ -z "$solve" ] && { echo "error: no 'time: total' line in the $label run's output:" >&2; echo "$out" >&2; exit 1; }
  [ -z "$node_ms" ] && node_ms="null"
  results="$results{\"label\":\"$label\",\"solve_ms\":$solve,\"node_ms\":$node_ms,\"process_ms\":$((t1 - t0)),\"computed\":${computed:-0},\"from_cache\":${hits:-0}},"
  echo "$label: solve $solve ms, $node $node_ms ms, process $((t1 - t0)) ms ($computed computed, $hits cached)" >&2
}

for i in $(seq 1 "$runs"); do
  cache="$scratch/cache-$i"
  one_run "$cache" "cold-$i"
  one_run "$cache" "warm-$i"
done

# best/median via sort (numeric); median = middle element (upper for even).
stat() { # stat <label-prefix> <field>
  local vals
  vals=$(printf '%s' "$results" | tr '}' '\n' | grep "\"label\":\"$1" | sed -n "s/.*\"$2\":\([0-9.]*\).*/\1/p" | sort -g)
  local n best median
  n=$(printf '%s\n' "$vals" | grep -c .)
  best=$(printf '%s\n' "$vals" | head -1)
  median=$(printf '%s\n' "$vals" | sed -n "$(( n / 2 + 1 ))p")
  printf '{"best":%s,"median":%s}' "${best:-null}" "${median:-null}"
}

json=$(printf '{"harness":"carve","pipeline":"%s","node":"%s","bin":"%s","runs":%s,"threads":%s,"cold":{"solve_ms":%s,"node_ms":%s,"process_ms":%s},"warm":{"solve_ms":%s,"node_ms":%s,"process_ms":%s},"target":{"cold_solve_ms_lt":10000,"warm_solve_ms_lt":100},"runs_detail":[%s]}' \
  "$pipeline" "$node" "$(printf '%s' "$bin" | sed 's/\\/\\\\/g')" "$runs" "${CICADA_THREADS:-0}" \
  "$(stat cold solve_ms)" "$(stat cold node_ms)" "$(stat cold process_ms)" \
  "$(stat warm solve_ms)" "$(stat warm node_ms)" "$(stat warm process_ms)" \
  "${results%,}")
printf '%s\n' "$json"
if [ -n "${CICADA_MEASURE_OUT:-}" ]; then printf '%s\n' "$json" > "$CICADA_MEASURE_OUT"; fi
printf 'carve %s --node %s ×%s: cold solve best/median %s ms, warm solve best/median %s ms (targets: cold < 10000 ms, warm < 100 ms); process wall cold %s ms / warm %s ms\n' \
  "$pipeline" "$node" "$runs" \
  "$(stat cold solve_ms | sed 's/[{}"a-z:]//g; s/,/\//')" "$(stat warm solve_ms | sed 's/[{}"a-z:]//g; s/,/\//')" \
  "$(stat cold process_ms | sed 's/[{}"a-z:]//g; s/,/\//')" "$(stat warm process_ms | sed 's/[{}"a-z:]//g; s/,/\//')"
