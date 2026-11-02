# Handover: vstack (2026-11-02)

## Status

vstack holds the shared Rust libraries for video switching and camboxes, on the model of dstack. It owns its code and releases, and apps pin its tags. It split from `zbynekdrlik/camera-box` main 3d06cb46f8 with history, under ticket airuleset#1280. The owner decided this on 2026-11-02 (camera-box#1487).

Imported: `jxs/link` (crate `camera-box-jxs`), `jxs/sys`, `jxs/recv-ffi`, `jxs/send-ffi`, `vendor/svt-jpeg-xs`, `rig/core` (crate `rig-core`). Nothing else yet.

The tree does not build yet: the workspace root stayed in camera-box. First work: add a workspace, CI and a tag, then tell camera-box and obs.

The clock stays with dsync. No clock code goes here. `rig/core` carries the genlock grid authority that the OBS C code mirrors.

Direction: vstack, then obs and the cambox app, then camera-box as orchestrator. A Rust editing app `vswitcher` built on vstack replaces the OBS fork later.

## Machines

- dev1: this project's account, with no sudo. Ask airuleset for build tools by peer.

## What else acts on the machines

- camera-box and obs pin your tags. You do not deploy anything.

## How to run a test

`cargo test` in the crates after the workspace exists.

## Site facts

Never commit a secret. Private facts go in `~/.config/vstack/`.

## Runners

None registered. camera-box CI built this code before.

## Backlog

Owner-requested:

- Hold the shared video libraries as a project (2026-11-02).

Proposed by a session, not yet asked for by the owner: nothing.

## Knowledge left in other projects

- camera-box: camera-box#1487, `docs/genlock-sender-contract.md`.
- airuleset: airuleset#1280.
