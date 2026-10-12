# Development

Read `.agents/conventions.md` before changing this repository.

## Purpose

`bake-agent-context` provides Bake tasks and a Rust API for discovering context directories in resolved Cargo dependencies, copying them into `.agents/context/`, and installing dependency skills into `.agents/skills/`.

## Important behavior

- Context providers place files in a top-level `context/` directory and include it in the published crate archive.
- Ordinary context is grouped by crate name beneath `.agents/context/`; skill-marked documents and their assets are installed only under `.agents/skills/`.
- Workspace members are skipped by default; resolved dependency packages are scanned.
- YAML front matter with `metadata.type: skill` opts a root-level document into skill installation. Its authored name and description are preserved; missing names derive from the package name and source filename.
- Installed `SKILL.md` front matter records `metadata.ecosystem`, `metadata.package`, and `metadata.version`. These fields identify provider ownership during refresh and stale-skill reconciliation.
- `ContextDocument` parses front matter through the Markdown AST and retains body bytes. `PackageContext` classifies loaded provider files once, keeping companion resources opaque. `Installer::install` coordinates direct writes and returns names for task presentation.
- Task names keep the Ruby-compatible `agent:context:*` namespace.
- `cargo metadata` is run from the project root and cached within a Bake task chain.

## Verification

Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, and `cargo test --workspace --locked`.
