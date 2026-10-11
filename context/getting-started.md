# Getting Started

This guide shows how to add Bake Agent Context tasks to a Rust project and install context from its Cargo dependencies.

## Add the Bake tasks

Bake tasks belong in your project's private `bake/` crate. Bootstrap the task package, add Bake Agent Context, and regenerate the task links:

```sh
cargo install socketry-cargo-bake --locked
cargo bake --regenerate
cargo add --manifest-path bake/Cargo.toml bake-agent-context
cargo bake --regenerate
```

Regeneration links the dependency's task registrations while preserving your task source; no manual import is needed.

## Install dependency context

Run this from the project root:

```sh
cargo bake agent:context:install
```

The task scans resolved Cargo dependencies for a top-level `context/` directory, copies ordinary context files into `.agents/context/SELECTOR/`, and installs skill-marked documents and their resources into `.agents/skills/`. Discover installed guidance by listing Markdown files under the provider's context directory and reading the guides relevant to the task.

Use `cargo bake agent:context:list` to see which dependencies provide ordinary context. Install context and skills from one provider with `cargo bake agent:context:install --package CRATE`, or inspect one of its context files with `cargo bake agent:context:show --package CRATE --file getting-started`.

List skills provided by dependencies with `cargo bake agent:context:skill:list`. The main install task installs all of them; use `cargo bake agent:context:skill:install --package CRATE --skill CRATE-SKILL` to install a specific skill independently. The skill name is prefixed with the provider crate name.

## Generated files

The install task maintains a marked block in the repository's local Git exclude file (`.git/info/exclude`) for installed context and dependency skills. Project-owned skill directories remain trackable. Repository instructions can direct agents to read relevant Markdown files under `.agents/context/`.

Each installed skill directory contains `skill.json` with its provider ecosystem, package, and version. Cargo refreshes reconcile Cargo-owned skills and reject conflicting ownership. See the portable specification for the shared format.
