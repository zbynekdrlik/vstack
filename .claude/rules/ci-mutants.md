---
paths:
  - ".github/workflows/**"
  - ".cargo/mutants.toml"
  - "Cargo.toml"
---

# CI and mutation testing (vstack#6)

- `cargo mutants --shard k/n` counts from 0: k runs 0..n-1, and `n/n` exits 1.
- The diff gate runs 4 shards (`mutants-shard`) and one gathering job named `Mutation Testing (diff)`,
  the required check. Before a large move, run the gate locally on the batch diff: a moved module
  often has mutants only camera-box's parity tests (which stay there) killed. Kill them with unit
  tests in vstack, or split the batch across tags. Never lower the cap.
- CI pins `cargo-mutants@27.1.0`. Run the same version locally (`~/.local/bin/cargo-mutants`,
  the release tarball): 25.x lacks mutant kinds such as "delete field … from struct expression",
  so an older local run passes what CI fails. Bump the pin and the local copy together.
- A missed mutant that no test can catch is excluded in `.cargo/mutants.toml` with an anchored
  `exclude_re` and a comment proving it is equivalent. Pin it to `line:col` when the same function
  has caught mutants, and diff `--list` with and without `--no-config` to prove only it is gone.
  Otherwise write the test.
- cargo-mutants builds in copies of the tree with a cold target dir: `Swatinem/rust-cache` does
  nothing in a mutants job.
- `--baseline=skip` only where a job before it proved the tree green (`needs: [test]`). The
  on-demand full sweep runs the baseline, so exit 4 fails it instead of passing vacuously.
- A change that touches only `#[cfg(test)]` or non-Rust files gives "No mutants to filter", exit 0.
  Prove a new mutants config on a real diff locally (`git diff <old> -- file > d.diff`,
  `cargo mutants --in-diff d.diff --all-features -- --all-targets`).
- Root `[profile.*]` applies only to builds inside a vstack checkout (camera-box and obs build
  the FFI archive there). Cargo ignores it for an app that pulls vstack as a git dependency.
- Each new required CI job: ask airuleset by peer to add it to the `main requires CI` ruleset.
- A new workspace crate gets its own `cargo nextest run --locked -p <crate>` step in the `Test`
  job (each crate is tested alone, as an app that pins it builds it). The coverage job's
  `--workspace` run does not replace it.
- A moved test can pass for the wrong reason. When a test asserts `None` from a function with
  several early returns, check which return fires. vstack#8 batch 3 found a band test that
  `square_span` rejected before the band check ran.
