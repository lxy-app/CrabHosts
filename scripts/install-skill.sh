#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "usage: $0 <harness-skills-directory> [--copy]" >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
source_dir="$repo_root/skills/crabhosts"
target_root="$1"
mode="${2:---link}"

if [[ ! -f "$source_dir/SKILL.md" ]]; then
  echo "error: canonical skill not found at $source_dir/SKILL.md" >&2
  exit 1
fi
if [[ "$mode" != "--link" && "$mode" != "--copy" ]]; then
  echo "error: second argument must be --link or --copy" >&2
  exit 2
fi

mkdir -p "$target_root"
target="$target_root/crabhosts"
if [[ -e "$target" || -L "$target" ]]; then
  echo "error: $target already exists; refusing to overwrite it" >&2
  exit 3
fi

if [[ "$mode" == "--copy" ]]; then
  cp -R "$source_dir" "$target"
else
  ln -s "$source_dir" "$target"
fi
printf 'Installed CrabHosts skill at %s\n' "$target"
