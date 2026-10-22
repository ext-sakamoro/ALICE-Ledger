#!/usr/bin/env bash
# scripts/preflight.sh — local reproduction of the CI gates before `git push`.
# Every command is the one CI runs. A step this file does not cover is a step
# that can only fail remotely — when a workflow changes, regenerate this file
# in the same commit (checks no workflow runs go in scripts/preflight.local.sh).
# Run `--quick` before every push.
#
# usage: scripts/preflight.sh [--quick]   (--quick skips the test / bench suites)
# generator rev e1525210a18e647fc6afa3496f68c5439fd7727d
set -euo pipefail
cd "$(dirname "$0")/.."
# --- heavy slot (begin) ---
# one heavy run at a time on this machine: a lock the kernel holds (flock on $ALICE_HEAVY_SLOT_DIR/lock, default
# $HOME/.cache/alice-heavy-slot) for as long as the holder lives. A small keeper process (perl) takes it and exits
# within a second of the holder's death, so a killed holder frees the slot and no child of the holder keeps it.
# `info` only names the holder for the waiting line. A wait past ALICE_HEAVY_SLOT_TIMEOUT seconds (default 7200)
# fails; the holder's own nested runs (ALICE_HEAVY_SLOT_HELD) run inside it.
heavy_slot_dir() { printf '%s\n' "${ALICE_HEAVY_SLOT_DIR:-$HOME/.cache/alice-heavy-slot}"; }
heavy_slot_acquire() {
  local label=$1 tool=$2 d line info err limit="${ALICE_HEAVY_SLOT_TIMEOUT:-7200}"
  d=$(heavy_slot_dir)
  if [ -n "${ALICE_HEAVY_SLOT_HELD:-}" ] && [ "$(sed -n 1p "$d/info" 2>/dev/null)" = "$ALICE_HEAVY_SLOT_HELD" ] \
     && kill -0 "$ALICE_HEAVY_SLOT_HELD" 2>/dev/null; then
    return 0
  fi
  if ! err=$(mkdir -p "$d" 2>&1) || ! err=$({ : >> "$d/lock"; } 2>&1); then
    echo "heavy slot: cannot create $d: ${err:-not writable}" >&2
    return 1
  fi
  exec 8< <(exec perl -MFcntl=:flock -e '
    my ($file, $limit) = @ARGV; my $parent = getppid(); $| = 1;
    open(my $fh, ">>", $file) or do { print "error $!\n"; exit 2 };
    print "pid $$\n";
    my $t = 0;
    until (flock($fh, LOCK_EX | LOCK_NB)) {
      if ($t >= $limit) { print "timeout\n"; exit 1 }
      print "wait $t\n" if $t % 30 == 0;
      sleep 1; $t++;
      exit 3 if getppid() != $parent;
    }
    print "held\n"; close(STDOUT);
    sleep 1 while getppid() == $parent;
  ' "$d/lock" "$limit")
  while IFS= read -r line <&8; do
    case "$line" in
      "pid "*) HEAVY_SLOT_KEEPER=${line#pid } ;;
      "wait "*)
        info=$(cat "$d/info" 2>/dev/null || true)
        echo "heavy slot: waiting for $(printf '%s\n' "$info" | sed -n 3p) pid $(printf '%s\n' "$info" | sed -n 1p) ($(printf '%s\n' "$info" | sed -n 4p))" >&2
        ;;
      held)
        exec 8<&-
        printf '%s\n%s\n%s\n%s\n' "$$" "$(date +%s)" "$label" "$tool" > "$d/info"
        ALICE_HEAVY_SLOT_HELD=$$
        export ALICE_HEAVY_SLOT_HELD
        return 0
        ;;
      timeout)
        exec 8<&-
        info=$(cat "$d/info" 2>/dev/null || true)
        echo "heavy slot: waited ${limit}s for $(printf '%s\n' "$info" | sed -n 3p) pid $(printf '%s\n' "$info" | sed -n 1p); giving up (ALICE_HEAVY_SLOT_TIMEOUT=$limit)" >&2
        return 1
        ;;
      "error "*)
        exec 8<&-
        echo "heavy slot: cannot open $d/lock: ${line#error }" >&2
        return 1
        ;;
    esac
  done
  exec 8<&-
  echo "heavy slot: the lock keeper stopped (is perl on PATH?)" >&2
  return 1
}
heavy_slot_release() {
  local d
  d=$(heavy_slot_dir)
  if [ "${ALICE_HEAVY_SLOT_HELD:-}" = "$$" ] && [ -n "${HEAVY_SLOT_KEEPER:-}" ]; then
    # the info first: once the keeper is gone the next holder writes its own
    rm -f "$d/info"
    kill "$HEAVY_SLOT_KEEPER" 2>/dev/null || true
    HEAVY_SLOT_KEEPER=
  fi
}
# --- heavy slot (end) ---
heavy_slot_acquire "$(basename "$PWD")" preflight || exit 1
trap heavy_slot_release EXIT

quick=0
[[ "${1:-}" == "--quick" ]] && quick=1

step() { printf '\n\033[1;34m== %s\033[0m\n' "$*"; }
# `cargo clippy` reuses fresh `cargo check` artifacts and then lints nothing;
# touching the crate roots invalidates only this repo's fingerprints.
relint() { git ls-files | grep -E '(^|/)src/(lib|main)\.rs$' | xargs -r touch; }
need() { command -v "$1" >/dev/null 2>&1 || { echo "missing tool: $1 ($2)" >&2; exit 1; }; }
has_toolchain() { rustup toolchain list | grep -q "^$1"; }

need actionlint "brew install actionlint"

step "ci.yml / fmt: Check formatting"
( export CARGO_TERM_COLOR="always"; cargo fmt -- --check )

step "ci.yml / actionlint: actionlint"
actionlint .github/workflows/*.yml

if [[ $quick -eq 1 ]]; then
  echo; echo "preflight --quick OK (test / bench suites skipped)"; exit 0
fi

step "ci.yml / test: Test (lib, integration, doctests)"
(
  export CARGO_TERM_COLOR="always" MIN_PASSED="168"
  # the default shell has no pipefail: without it cargo's exit status is lost in the pipe
  set -o pipefail
  log="$(mktemp)"
  cargo test --all-features 2>&1 | tee "${log}"
  passed=$(grep -E '^test result: ' "${log}" | awk '{s += $4} END {print s + 0}')
  failed=$(grep -E '^test result: ' "${log}" | awk '{s += $6} END {print s + 0}')
  echo "passed: ${passed} (minimum ${MIN_PASSED}), failed: ${failed}"
  if [ "${failed}" -ne 0 ] || grep -E '^test result: ' "${log}" | grep -vE '^test result: ok\. ' >/dev/null; then
    echo "::error::${failed} tests failed"
    exit 1
  fi
  if [ "${passed}" -lt "${MIN_PASSED}" ]; then
    echo "::error::${passed} tests passed, fewer than ${MIN_PASSED}"
    exit 1
  fi
)

echo; echo "preflight OK"
