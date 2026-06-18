#!/usr/bin/env bash
# SPDX-License-Identifier: GLWTPL

set -Eeuo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ -x "$script_dir/target/release/kiss-me" ]]; then
	exec "$script_dir/target/release/kiss-me" "$@"
fi

if [[ -x "$script_dir/target/debug/kiss-me" ]]; then
	exec "$script_dir/target/debug/kiss-me" "$@"
fi

exec cargo run --quiet --manifest-path "$script_dir/Cargo.toml" -- "$@"

