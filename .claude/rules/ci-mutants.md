---
paths:
  - ".github/workflows/**"
  - ".cargo/mutants.toml"
  - "Cargo.toml"
---

# CI and mutation testing (vstack#6)

- `cargo mutants --shard k/n` counts from 0: k runs 0..n-1, and `n/n` exits 1.
- A missed mutant that no test can catch is excluded in `.cargo/mutants.toml` with an anchored
  `exclude_re` (`^path:[0-9]+:[0-9]+: replace … in fn$`) and a comment proving it is equivalent.
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
