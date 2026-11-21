#!/bin/bash
# Run one Rust test repeatedly with the CPUs kept busy, to shake out timing-dependent flakes.
#
#   bin=$(cargo test -p ide-lsp --test mock_server --no-run --message-format=json 2>/dev/null \
#     | grep -o '"executable":"[^"]*mock_server[^"]*"' | cut -d'"' -f4 | tail -1)
#   scripts/stress-test.sh "$bin" <test-name> [runs=60] [load-processes=4]
#
# The load is stopped by PID from a trap, so it goes however the script ends. `kill %1` would
# not do: a script has no job control, so job specs match nothing and every `yes` outlives it.
set -u

if [ $# -lt 2 ]; then
    echo "usage: $0 <test-binary> <test-name> [runs] [load-processes]" >&2
    exit 2
fi

bin=$1
name=$2
runs=${3:-60}
load=${4:-4}

pids=()
stop_load() {
    ((${#pids[@]})) || return 0
    kill "${pids[@]}" 2>/dev/null
    wait "${pids[@]}" 2>/dev/null
    pids=()
}
trap stop_load EXIT
trap 'exit 130' INT TERM

for _ in $(seq 1 "$load"); do
    yes >/dev/null &
    pids+=($!)
done

fails=0
for _ in $(seq 1 "$runs"); do
    "$bin" --exact "$name" -q >/dev/null 2>&1 || fails=$((fails + 1))
done

echo "under load: $fails/$runs failed"
[ "$fails" -eq 0 ]
