#!/usr/bin/env bash
set -euo pipefail

root_directory="$(cd "$(dirname "$0")/.." && pwd -P)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT

fixture="$temporary_directory/fixture"
fake_tools="$temporary_directory/tools"
mkdir -p "$fixture/scripts" "$fixture/crates/rusttable-app" "$fake_tools"
cp "$root_directory/scripts/linux-distribution-smoke.sh" "$fixture/scripts/"
cp "$root_directory/scripts/linux-artifact-identity.ts" "$fixture/scripts/"
cp "$root_directory/scripts/with-validation-budget.sh" "$fixture/scripts/"
cp "$root_directory/LICENSE" "$fixture/LICENSE"
touch "$fixture/crates/rusttable-app/Cargo.toml"

cat >"$fake_tools/uname" <<'EOF'
#!/bin/sh
printf 'Linux\n'
EOF
cat >"$fake_tools/git" <<'EOF'
#!/bin/sh
if [ "$1 $2" = 'rev-parse --show-toplevel' ]; then
  printf '%s\n' "$FAKE_ROOT"
elif [ "$1" = '-C' ] && [ "$3 $4" = 'rev-parse HEAD' ]; then
  printf 'fixture-git-sha\n'
else
  exec /usr/bin/git "$@"
fi
EOF
cat >"$fake_tools/cargo" <<'EOF'
#!/bin/sh
if [ "$1 $2 $3 $4" = 'metadata --locked --no-deps --format-version' ]; then
  printf '{"packages":[{"name":"rusttable-app","manifest_path":"%s/crates/rusttable-app/Cargo.toml","version":"0.1.0"}]}\n' "$FAKE_ROOT"
  exit 0
fi
if [ "$1" = build ]; then
  printf '%s\n' "$*" >"$FAKE_LOG"
  [ "$FAKE_FAIL_BUILD" = 1 ] && exit 77
  [ "$FAKE_SLEEP_BUILD" = 1 ] && sleep 5
  mkdir -p "$FAKE_ROOT/target/release"
  printf '#!/bin/sh\n[ "$1" = --version ] && printf "RustTable 0.1.0\\n"\n' >"$FAKE_ROOT/target/release/rusttable-app"
  chmod 755 "$FAKE_ROOT/target/release/rusttable-app"
  exit 0
fi
exit 78
EOF
cat >"$fake_tools/bun" <<'EOF'
#!/bin/sh
if [ "$1" = -e ]; then
  input="$(cat)"
  case "$input" in
    *RUSTTABLE_LINUX_DISTRIBUTION_V1*)
      printf 'rust_release=1.95.0\nrust_host=x86_64-unknown-linux-gnu\nelf_class=ELF64\nelf_data=2'\''s complement, little endian\nelf_machine=Advanced Micro Devices X86-64\nelf_type=DYN\nprogram_interpreter=/lib64/ld-linux-x86-64.so.2\nneeded_library=libc.so.6\n'
      ;;
    *) printf '0.1.0' ;;
  esac
  exit 0
fi
if [ "$1" = scripts/linux-artifact-identity.ts ]; then
  cat <<'JSON'
{"schema":"RUSTTABLE_LINUX_DISTRIBUTION_V1","packageVersion":"0.1.0","archiveBasename":"RustTable-0.1.0-x86_64-unknown-linux-gnu-unsigned.tar.gz","rustRelease":"1.95.0","rustHost":"x86_64-unknown-linux-gnu","elfClass":"ELF64","elfData":"2's complement, little endian","elfMachine":"Advanced Micro Devices X86-64","elfType":"DYN","interpreter":"/lib64/ld-linux-x86-64.so.2","needed":["libc.so.6"]}
JSON
  exit 0
fi
exit 79
EOF
cat >"$fake_tools/rustc" <<'EOF'
#!/bin/sh
cat <<'OUT'
rustc 1.95.0 (fixture)
release: 1.95.0
host: x86_64-unknown-linux-gnu
OUT
EOF
cat >"$fake_tools/readelf" <<'EOF'
#!/bin/sh
case "$1" in
  -h) cat <<'OUT'
ELF Header:
  Magic:   7f 45 4c 46 02 01 01 00
  Class: ELF64
  Data: 2's complement, little endian
  Type: DYN (Position-Independent Executable file)
  Machine: Advanced Micro Devices X86-64
OUT
    ;;
  -l) printf '[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]\n' ;;
  -d) printf '0x1 (NEEDED) Shared library: [libc.so.6]\n' ;;
  *) exit 80 ;;
esac
EOF
cat >"$fake_tools/ldd" <<'EOF'
#!/bin/sh
printf 'libc.so.6 => /lib64/libc.so.6 (0x0000)\n'
EOF
cat >"$fake_tools/tar" <<'EOF'
#!/bin/sh
if [ "$1" = --version ]; then
  printf 'tar (GNU tar) 1.35\n'
  exit 0
fi
if [ "$1" = --extract ]; then
  directory=''
  previous=''
  for argument in "$@"; do
    [ "$previous" = --directory ] && directory="$argument"
    previous="$argument"
  done
  top="$directory/RustTable-0.1.0-x86_64-unknown-linux-gnu"
  mkdir -p "$top/bin"
  cp "$FAKE_ROOT/target/release/rusttable-app" "$top/bin/RustTable"
  cp "$FAKE_ROOT/LICENSE" "$top/LICENSE"
  chmod 755 "$top/bin/RustTable"
  chmod 644 "$top/LICENSE"
  exit 0
fi
if [ "$1" = -tzf ]; then
  printf 'RustTable-0.1.0-x86_64-unknown-linux-gnu/\nRustTable-0.1.0-x86_64-unknown-linux-gnu/LICENSE\nRustTable-0.1.0-x86_64-unknown-linux-gnu/bin/\nRustTable-0.1.0-x86_64-unknown-linux-gnu/bin/RustTable\n'
  exit 0
fi
printf 'deterministic fake tar stream\n'
EOF
cat >"$fake_tools/gzip" <<'EOF'
#!/bin/sh
cat
EOF
cat >"$fake_tools/sha256sum" <<'EOF'
#!/bin/sh
if [ "$1" = -c ]; then
  exit 0
fi
printf '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  %s\n' "$(basename "$1")"
EOF
cat >"$fake_tools/stat" <<'EOF'
#!/bin/sh
case "$2" in
  %a) case "$3" in *RustTable) printf '755\n' ;; *) printf '644\n' ;; esac ;;
  %s) case "$3" in *.tar.gz) printf '24\n' ;; *) printf '7\n' ;; esac ;;
  *) exit 81 ;;
esac
EOF
chmod +x "$fake_tools"/*

run_smoke() {
  PATH="$fake_tools:$PATH" FAKE_ROOT="$fixture" FAKE_LOG="$temporary_directory/build-args" "$@"
}

archive="$fixture/target/linux-distribution/RustTable-0.1.0-x86_64-unknown-linux-gnu-unsigned.tar.gz"
run_smoke bash "$fixture/scripts/linux-distribution-smoke.sh"
[[ "$(cat "$temporary_directory/build-args")" = 'build --release --package rusttable-app --bin rusttable-app --locked' ]]
[[ -s "$fixture/target/linux-distribution/smoke.log" ]]
[[ -f "$archive" && -f "$archive.sha256" ]]
[[ ! -e "$fixture/target/linux-distribution/.work" ]]
grep -q '^pass=archive-checksum$' "$fixture/target/linux-distribution/smoke.log"
grep -q '^pass=smoke-complete$' "$fixture/target/linux-distribution/smoke.log"

rm -rf "$fixture/target/linux-distribution"
if run_smoke env FAKE_FAIL_BUILD=1 bash "$fixture/scripts/linux-distribution-smoke.sh"; then
  printf 'expected build failure fixture to fail\n' >&2
  exit 1
else
  failure_status=$?
fi
[[ "$failure_status" -ne 0 ]]
[[ ! -e "$fixture/target/linux-distribution/RustTable-0.1.0-x86_64-unknown-linux-gnu-unsigned.tar.gz" ]]
[[ ! -e "$fixture/target/linux-distribution/smoke.log" ]]

rm -rf "$fixture/target/linux-distribution"
if run_smoke env FAKE_SLEEP_BUILD=1 bash "$fixture/scripts/with-validation-budget.sh" 1 linux-distribution-smoke bash "$fixture/scripts/linux-distribution-smoke.sh"; then
  printf 'expected timeout fixture to fail\n' >&2
  exit 1
else
  timeout_status=$?
fi
[[ "$timeout_status" -eq 124 ]]
sleep 1
[[ ! -e "$fixture/target/linux-distribution/RustTable-0.1.0-x86_64-unknown-linux-gnu-unsigned.tar.gz" ]]
[[ ! -e "$fixture/target/linux-distribution/smoke.log" ]]

printf 'linux distribution smoke fake fixtures passed\n'
