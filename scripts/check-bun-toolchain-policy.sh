#!/usr/bin/env bash
set -euo pipefail

if (( $# > 1 )); then
  printf 'usage: %s [REPOSITORY_ROOT]\n' "$0" >&2
  exit 2
fi

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
if (( $# == 1 )); then
  repository_root="$1"
else
  repository_root="$(cd "$script_directory/.." && pwd -P)"
fi

if [[ ! -d "$repository_root" ]]; then
  printf 'bun policy: repository root is not a directory: %s\n' "$repository_root" >&2
  exit 1
fi

status=0
for workflow in .github/workflows/rust-pr.yml .github/workflows/rust-main.yml; do
  workflow_path="$repository_root/$workflow"
  if [[ ! -f "$workflow_path" ]]; then
    printf 'bun policy: required workflow is missing: %s\n' "$workflow" >&2
    status=1
    continue
  fi
  if ! awk -v workflow="$workflow" '
    function report(message) {
      printf "bun policy: workflow %s job %s: %s\n", workflow, job_name, message > "/dev/stderr"
      violations++
    }

    function is_bun_command(line) {
      if (line ~ /oven-sh\/setup-bun@/ || line ~ /scripts\/bun-pin\.sh/ || line ~ /bun-version:/) return 0
      return line ~ /(^|[^[:alnum:]_-])bun([^[:alnum:]_-]|$)/
    }

    function action_ref(text, action, remainder) {
      action = text
      sub("^.*oven-sh/setup-bun@", "", action)
      remainder = action
      sub(/[[:space:]].*$/, "", remainder)
      return remainder
    }

    function immutable_setup(text, reference) {
      reference = action_ref(text)
      return reference ~ /^[0-9A-Fa-f]+$/ && length(reference) == 40
    }

    function has_canonical_output(text) {
      return text ~ /bun-version:[[:space:]]*\$\{\{[[:space:]]*steps\.bun-pin\.outputs\.bun-version[[:space:]]*\}\}/
    }

    function has_package_hash(text) {
      return text ~ /hashFiles\([^)]*package\.json/
    }

    function check_job(    step_index, inner_index, line_count, line_parts, step_line, command_count, command_step, has_setup, has_pin, has_verify, has_cache, setup_output_count, immutable_count, checkout_step, pin_step, setup_step, verify_step, cache_count, package_hash_count, pin_reads, invocation_before_verify, step_name, step_has_bun, step_has_pin, step_has_setup, step_has_verify, step_has_cache, inner_line) {
      if (job_name == "") return
      has_setup = 0
      has_pin = 0
      has_verify = 0
      has_cache = 0
      setup_output_count = 0
      immutable_count = 0
      checkout_step = 0
      pin_step = 0
      setup_step = 0
      verify_step = 0
      cache_count = 0
      package_hash_count = 0
      pin_reads = 0
      command_count = 0
      command_step = 0

      for (step_index = 1; step_index <= step_count; step_index++) {
        step_name = step_text[step_index]
        sub(/\n.*$/, "", step_name)
        sub(/^.*- name:[[:space:]]*/, "", step_name)
        step_has_pin = step_text[step_index] ~ /scripts\/bun-pin\.sh/
        step_has_setup = step_text[step_index] ~ /oven-sh\/setup-bun@/
        step_has_verify = step_name ~ /Verify canonical Bun version/ && step_text[step_index] ~ /bun --version/ && step_text[step_index] ~ /steps\.bun-pin\.outputs\.bun-version/
        step_has_cache = step_text[step_index] ~ /uses:[[:space:]]*actions\/cache@/

        if (step_text[step_index] ~ /actions\/checkout@/) checkout_step = step_index
        if (step_has_pin) {
          has_pin++
          pin_step = pin_step == 0 ? step_index : pin_step
        }
        if (step_has_setup) {
          has_setup++
          setup_step = setup_step == 0 ? step_index : setup_step
          if (immutable_setup(step_text[step_index])) immutable_count++
          if (has_canonical_output(step_text[step_index])) setup_output_count++
        }
        if (step_has_verify) {
          has_verify++
          verify_step = verify_step == 0 ? step_index : verify_step
        }
        if (step_has_cache) {
          cache_count++
          if (has_package_hash(step_text[step_index])) package_hash_count++
        }

        line_count = split(step_text[step_index], line_parts, "\n")
        for (inner_index = 1; inner_index <= line_count; inner_index++) {
          inner_line = line_parts[inner_index]
          pin_reads += gsub(/scripts\/bun-pin\.sh/, "&", inner_line)
          if (is_bun_command(inner_line)) {
            command_count++
            if (command_step == 0) command_step = step_index
          }
        }
      }

      if (has_setup == 0 && command_count == 0) return
      if (checkout_step == 0) report("checkout step is missing")
      if (has_pin == 0) report("canonical pin step is missing")
      if (pin_reads != 1) report("canonical pin is read " pin_reads " times")
      if (has_setup == 0) report("Bun setup step is missing")
      if (has_setup > 1) report("Bun setup step is duplicated")
      if (has_setup > 0 && immutable_count != has_setup) report("Bun setup action must use an immutable commit pin")
      if (has_setup > 0 && setup_output_count != has_setup) report("Bun version selection must use canonical output")
      if (has_verify == 0) report("verification step is missing")
      if (has_verify > 1) report("verification step is duplicated")
      for (step_index = 1; step_index <= step_count; step_index++) {
        if (step_text[step_index] ~ /uses:[[:space:]]*actions\/cache@/ && !has_package_hash(step_text[step_index])) {
          report("cache key must hash package.json")
        }
      }

      if (checkout_step == 0 || pin_step == 0 || setup_step == 0 || verify_step == 0 || pin_step <= checkout_step || setup_step <= pin_step || verify_step != setup_step + 1) {
        report("Bun setup sequence is incomplete or out of order")
      }
      invocation_before_verify = 0
      if (verify_step > 0) {
        for (step_index = 1; step_index < verify_step; step_index++) {
          line_count = split(step_text[step_index], line_parts, "\n")
          for (inner_index = 1; inner_index <= line_count; inner_index++) {
            if (is_bun_command(line_parts[inner_index])) invocation_before_verify++
          }
        }
      }
      if (invocation_before_verify > 0) report("Bun is invoked before version verification")
    }

    function reset_job(    step_index) {
      for (step_index = 1; step_index <= step_count; step_index++) delete step_text[step_index]
      for (step_index = 1; step_index <= step_count; step_index++) delete step_start[step_index]
      step_count = 0
      current_step = 0
      job_name = ""
    }

    BEGIN {
      in_jobs = 0
      job_name = ""
      step_count = 0
      current_step = 0
      violations = 0
    }

    /^jobs:[[:space:]]*$/ {
      in_jobs = 1
      next
    }

    in_jobs && /^[^[:space:]]/ {
      check_job()
      reset_job()
      in_jobs = 0
      next
    }

    in_jobs && /^  [A-Za-z0-9_-]+:[[:space:]]*$/ {
      check_job()
      reset_job()
      job_name = $0
      sub(/^  /, "", job_name)
      sub(/:.*/, "", job_name)
      next
    }

    in_jobs && job_name != "" {
      if ($0 ~ /^[[:space:]]+- name:/) {
        current_step++
        step_count = current_step
        step_start[current_step] = FNR
        step_text[current_step] = $0
      } else if (current_step > 0) {
        step_text[current_step] = step_text[current_step] "\n" $0
      }
    }

    END {
      check_job()
      if (violations > 0) exit 1
    }
  ' "$workflow_path"; then
    status=1
  fi
done

if (( status == 0 )); then
  printf 'bun policy: hosted workflow fixtures compliant\n'
fi
exit "$status"
