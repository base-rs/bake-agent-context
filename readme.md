# `bake-agent-context`

Bake tasks for discovering and installing context files shipped by Cargo dependencies. It is the Rust counterpart to the Ruby [`agent-context`](https://github.com/socketry/agent-context) gem.

## Setup

Add the task library to the unpublished `bake/` crate and regenerate its task links:

```sh
cargo bake --regenerate
cargo add --manifest-path bake/Cargo.toml bake-agent-context
cargo bake --regenerate
```

Install context and skills from all resolved dependencies:

```sh
cargo bake agent:context:install
```

Other tasks:

```sh
cargo bake agent:context:list
cargo bake agent:context:list --package socketry-executor
cargo bake agent:context:show --package socketry-executor --file getting-started
cargo bake agent:context:install --package socketry-executor
cargo bake agent:context:skill:list
cargo bake agent:context:skill:install
cargo bake agent:context:skill:install --package socketry-executor --skill socketry-executor-initial-gem-setup
```

`cargo bake agent:context:install` copies ordinary guidance to `.agents/context/SELECTOR/` and maintains local Git exclusions for installed context and dependency skills. Browse the installed Markdown files or use the list/show commands to find relevant guides. Project-owned skills can be tracked under `.agents/skills/`.

Context documents marked with `type: skill` are installed only under `.agents/skills/`, along with their companion resources. Their installed names are prefixed with the provider crate name, and they are omitted from `.agents/context/`. Use `agent:context:install --package CRATE` to install both context and skills from one provider. The separate skill tasks are useful for listing skills or installing a selected skill. Project-owned skills can live in `.agents/skills/` and remain version controlled.

## Provide context from a crate

Add a top-level `context/` directory to the crate and ensure Cargo packages it. When using an explicit `include` list in `Cargo.toml`, include `context/**`:

```text
my-crate/
├── Cargo.toml
├── context/
│   └── getting-started.md
└── src/
```

Use descriptive filenames and focused Markdown guides so agents can find relevant guidance in `.agents/context/SELECTOR/`. Ordinary context files need no Markdown front matter. A context document can opt into skill installation with YAML front matter; see [Using and Providing Context](context/usage.md), which is installed as the `bake-agent-context-usage` skill.

## Context

This crate includes guides for its users and for other crate authors:

- [Getting Started](context/getting-started.md) explains how to add and run the Bake tasks.
- [Using and Providing Context](context/usage.md) describes dependency context and how to publish context from a crate.
- [Agent Context](context/agent-context.md) explains how to organize package guidance and repository-only instructions.
- [Agent Context Specification](context/specification.md) defines the language-agnostic context directory and installation conventions.
- [Rust Context](context/rust.md) provides shared development guidance for Socketry's Rust crates.

## Releasing

Prepare a release with `cargo bake cargo:version:patch` (or `minor`, `major`, or `bump --version X.Y.Z`), then run `cargo bake cargo:release` and open a pull request. After review and merge, GitHub Actions publishes the release when the configured `crates-io` environment approves it. Follow the shared [Releasing skill](https://github.com/socketry/socketry-project-rust/blob/main/context/releasing.md) for the standard process.

## Releases

<!-- bake-readme:releases:start -->

See [releases.md](releases.md) for the full release history.

### v0.3.3

- Use Cargo-selected dependency versions and generated task links in setup instructions.

### v0.3.2

- Adopt `socketry-project` 0.3.7 for shared project tasks and Markdown normalization.
- Require the aggregate test and coverage result for pull request merges.
- Refresh dependency examples and repository-owned agent guidance.

### v0.3.1

- Require `socketry-markdown` 0.2.0 or newer for safe inline Markdown serialization.

<!-- bake-readme:releases:end -->

## See Also

- [`bake`](https://github.com/socketry/bake-rust).
- [`socketry-project`](https://github.com/socketry/socketry-project-rust).

## Contributing

Please open an issue or pull request on [GitHub](https://github.com/socketry/bake-agent-context-rust).

Read the [architecture design](context/design.md) for the current Ruby and Rust boundaries and the proposed alignment work.

### Agent Context

Run `cargo bake agent:context:install` to install shared context and skills. Follow the repository's agent instructions, discover relevant Markdown guides under `.agents/context/`, and apply matching skills under `.agents/skills/`.

Each installed skill directory contains `skill.json` with its provider ecosystem, package, and version. Cargo refreshes reconcile Cargo-owned skills and reject conflicting ownership. See the portable specification for the shared format.
