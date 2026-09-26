#!/usr/bin/env bash
# Sends one prompt to GitHub Copilot and writes the answer to a file.
#
#   bash .github/scripts/copilot-infer.sh SYSTEM_PROMPT_FILE USER_PROMPT_FILE OUTPUT_FILE
#
# Needs .github/actions/setup-copilot-cli (for `copilot-locked`), the job
# permission `copilot-requests: write`, and the workflow token in
# COPILOT_GITHUB_TOKEN. COPILOT_MODEL picks the model (default `auto`, the
# only value every Copilot plan accepts).
#
# GitHub Models (models.github.ai) was retired on 2026-07-30. It answered 410
# after that, and later 200 with a body that is not JSON, so every workflow
# that still called it degraded on every run. Copilot is what the workflow
# token can reach now.
#
# The prompt goes to the CLI as one argument, and Linux caps one argument at
# 128 KiB. The user prompt is cut to fit COPILOT_PROMPT_MAX_BYTES, so put the
# least important context at its end.
#
# Exits 0 with a non-empty answer in OUTPUT_FILE, 1 otherwise. The caller
# decides how to degrade. Nothing the model or the prompt says is printed to
# the log without `::stop-commands::` around it, because a log line that
# starts with `::` is a workflow command.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: $0 SYSTEM_PROMPT_FILE USER_PROMPT_FILE OUTPUT_FILE" >&2
  exit 2
fi

system_file=$1
user_file=$2
out_file=$3
model=${COPILOT_MODEL:-auto}
max_bytes=${COPILOT_PROMPT_MAX_BYTES:-120000}
timeout_seconds=${COPILOT_TIMEOUT_SECONDS:-300}

: > "$out_file"

if [ -z "${COPILOT_GITHUB_TOKEN:-}" ]; then
  echo "::error::COPILOT_GITHUB_TOKEN is not set"
  exit 1
fi
if ! command -v copilot-locked > /dev/null 2>&1; then
  echo "::error::copilot-locked is not on PATH; run .github/actions/setup-copilot-cli first"
  exit 1
fi

system_bytes=$(wc -c < "$system_file")
budget=$((max_bytes - system_bytes - 2))
if [ "$budget" -lt 1024 ]; then
  echo "::error::The system prompt ($system_bytes bytes) leaves no room for the user prompt"
  exit 1
fi
user_bytes=$(wc -c < "$user_file")
if [ "$user_bytes" -gt "$budget" ]; then
  echo "::warning::User prompt cut from $user_bytes to $budget bytes to fit one argument"
fi

scratch=$(mktemp -d)
workdir=$(mktemp -d)
trap 'rm -rf "$scratch" "$workdir"' EXIT

# NUL bytes cannot travel in an argument, and a cut can split a UTF-8
# sequence; iconv -c drops the broken tail (and exits 1 when it does).
{
  cat "$system_file"
  printf '\n\n'
  head -c "$budget" "$user_file" | tr -d '\000' | { iconv -f UTF-8 -t UTF-8 -c 2> /dev/null || true; }
} > "$scratch/prompt.txt"
prompt=$(cat "$scratch/prompt.txt")

# An empty working directory: the file tools that need no approval have
# nothing to read.
status=0
(
  cd "$workdir"
  timeout --kill-after=15 "$timeout_seconds" \
    copilot-locked -p "$prompt" -s --no-ask-user --model "$model"
) > "$out_file" 2> "$scratch/stderr.txt" || status=$?

if [ "$status" -ne 0 ]; then
  if [ "$status" -eq 124 ]; then
    echo "::error::Copilot CLI timed out after ${timeout_seconds}s"
  else
    echo "::error::Copilot CLI exited with status $status"
  fi
  if [ -s "$scratch/stderr.txt" ]; then
    stop_token="copilot-stderr-$(od -An -N12 -tx1 /dev/urandom | tr -d ' \n')"
    echo "::stop-commands::$stop_token"
    echo "Copilot CLI stderr (last 2000 bytes):"
    tail -c 2000 "$scratch/stderr.txt"
    echo
    echo "::$stop_token::"
  fi
  exit 1
fi

if [ -z "$(tr -d '[:space:]' < "$out_file")" ]; then
  echo "::error::Copilot CLI returned an empty answer"
  exit 1
fi

echo "Copilot answered with $(wc -c < "$out_file") bytes"
