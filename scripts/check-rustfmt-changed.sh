#!/usr/bin/env bash
set -euo pipefail

resolve_default_range() {
  if git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' >/dev/null 2>&1; then
    local upstream
    upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}')"
    local base
    base="$(git merge-base HEAD "$upstream")"
    printf '%s...HEAD\n' "$base"
    return
  fi

  if git rev-parse HEAD~1 >/dev/null 2>&1; then
    printf 'HEAD~1...HEAD\n'
    return
  fi

  local root
  root="$(git rev-list --max-parents=0 HEAD | tail -n 1)"
  printf '%s...HEAD\n' "$root"
}

range="${1:-}"
if [ -z "$range" ]; then
  range="$(resolve_default_range)"
fi

if ! git rev-parse "${range%%...*}" >/dev/null 2>&1; then
  range="$(resolve_default_range)"
fi

mapfile -t rust_files < <(git diff --name-only --diff-filter=ACMR "$range" -- '*.rs' | sort -u)
if [ "${#rust_files[@]}" -eq 0 ]; then
  echo "No changed Rust files to format-check in range: $range"
  exit 0
fi

existing_files=()
for file in "${rust_files[@]}"; do
  if [ -f "$file" ]; then
    existing_files+=("$file")
  fi
done

if [ "${#existing_files[@]}" -eq 0 ]; then
  echo "No existing Rust files to format-check in range: $range"
  exit 0
fi

echo "Running rustfmt --check on ${#existing_files[@]} changed Rust file(s)."
rustfmt --edition 2021 --check "${existing_files[@]}"
