# Handover: vstack (2026-11-02)

## Status

vstack holds the shared Rust libraries for video switching and camboxes, on the model of dstack. It owns its code and releases, and apps pin its tags. It split from `zbynekdrlik/camera-box` main 3d06cb46f8 with history, under ticket airuleset#1280. The owner decided this on 2026-11-02 (camera-box#1487).

Imported: `jxs/link` (crate `camera-box-jxs`), `jxs/sys`, `jxs/recv-ffi`, `jxs/send-ffi`, `vendor/svt-jpeg-xs`, `rig/core` (crate `rig-core`). From camera-box 3d06cb46f8 on camera-box's request (vstack#1): `genlock_backlog`, `genlock_n1_depth`, `genlock_wall_step` in `rig/core`; `qpsk_marker`, `qpsk_marker_scan` in `av/marker` (crate `qpsk-marker`). From camera-box dev 390ae50 (vstack#8, batch 1 for obs): `genlock_n2_grid`, `genlock_fifo_relabel`, `genlock_lock_state`, `genlock_jxs_state` in `rig/core` (v0.1.5); `genlock_audio_pairing`, `genlock_audio_step_hold`, `genlock_audio_buffering` in `rig/core` (v0.1.6). The benches on camera-box's `genlock_grid_bench` (and `genlock_audio_pairing_bench` with the shallow bench) stay in camera-box. The `asrc_*` family (vstack#8 batch 4) does not come here: the owner put the ASRC in `dstack-resample` in the dstack repo (dstack #90, ASRC from v0.6.2). rig-core keeps the placement side that feeds it (`AsrcBasis`, the error and rate-point helpers) and does not depend on dstack-resample; the apps join the two.

The root `Cargo.toml` is the workspace; every crate inherits its one version. Tags: `git ls-remote --tags origin` (v0.1.0 the import, v0.1.1 camera-box's modules, v0.1.2 CI hardening, v0.1.3 doc wording, v0.1.4 mutation gate and release profile, vstack#6; v0.1.5 and v0.1.6 vstack#8's batch 1, video then audio). The parity gates against the C mirrors stay in the apps that carry the C code; they test these crates.

The clock stays with dsync. No clock code goes here. `rig/core` carries the genlock grid authority that the OBS C code mirrors.

Direction: vstack, then obs and the cambox app, then camera-box as orchestrator. A Rust editing app `vswitcher` built on vstack replaces the OBS fork later.

## Machines

- dev1: this project's account, with no sudo. Ask airuleset for build tools by peer.

## What else acts on the machines

- camera-box and obs pin your tags. You do not deploy anything.

## How to run a test

`cargo nextest run --workspace --all-features` (or `cargo test`). Needs `cmake`, `nasm` and a C compiler for `jxs/sys`.

Mutation testing (vstack#6): CI's `Mutation Testing (diff)` mutates the lines a change touches, in 4 shards that each keep the 20 min cap, and fails on a missed mutant. Run it locally with `git diff origin/main...HEAD > /path/change.diff && cargo mutants --in-diff /path/change.diff --all-features -- --all-targets`. `.cargo/mutants.toml` holds the config and the equivalent-mutant excludes, each with its reason. The whole tree (about 4000 mutants) is mutated only on demand: `gh workflow run mutation-full.yml`, which files the missed mutants as one `test-quality` issue.

The root `[profile.release]` (LTO, one codegen unit, strip) is the profile of the FFI archive that camera-box and obs build in a vstack checkout. Cargo ignores it when an app pulls vstack in as a git dependency.

## Site facts

Never commit a secret. Private facts go in `~/.config/vstack/`.

## Runners

None registered. CI runs on GitHub-hosted `ubuntu-latest` (`.github/workflows/ci.yml`).

## Releases

Bump `[workspace.package] version` on dev first, then `cargo update -w` so `Cargo.lock` carries it (CI builds `--locked`, and fails dev if the version is not above main). A green main run tags `vX.Y.Z`; a tag never moves. Then tell camera-box and obs the tag by peer.

## Backlog

Owner-requested:

- Hold the shared video libraries as a project (2026-11-02).

Proposed by a session, not yet asked for by the owner: nothing.

## Knowledge left in other projects

- camera-box: camera-box#1487, `docs/genlock-sender-contract.md`.
- airuleset: airuleset#1280.
