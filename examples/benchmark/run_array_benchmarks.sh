#!/bin/bash
cd "$(dirname "$0")/../.." || exit 1

echo "Aelys vs python array benchmark"

cargo build --release -q 2>/dev/null

run_benchmark() {
    local name=$1
    local aelys_file=$2
    local py_file=$3

    echo "--- $name ---"
    echo "Aelys:"
    ./target/release/aelys-cli "$aelys_file" 2>&1 | grep -E "(time|Time)"
    echo "Python:"
    python3 "$py_file" 2>&1 | grep -E "(time|Time)"
    echo ""
}

run_benchmark "Array Sum (1M elements)" \
    "examples/benchmark/array_sum.aelys" \
    "examples/benchmark/array_sum.py"

run_benchmark "Vec Push/Pop (1M ops)" \
    "examples/benchmark/vec_pushpop.aelys" \
    "examples/benchmark/vec_pushpop.py"

run_benchmark "Bubble Sort (5K elements)" \
    "examples/benchmark/array_sort.aelys" \
    "examples/benchmark/array_sort.py"
