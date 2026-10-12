---
metadata:
  type: skill
description: Use this skill to install and navigate agent context and skills provided by Cargo dependencies.
---

# Using and Providing Context

Use Bake Agent Context to discover and install practical guidance shipped in dependency crates.

## Use dependency context

Context providers store agent-focused documentation in a `context/` directory at the root of their crate. Bake Agent Context scans resolved Cargo dependencies for this directory.

Run `cargo bake agent:context:list` to see available providers. Each provider is listed by crate name and version. If multiple versions of one crate provide context, select the desired version using the `crate@version` selector shown by the task.

Run `cargo bake agent:context:list --package CRATE` to list one provider's files. Read a file without installing it using `cargo bake agent:context:show --package CRATE --file FILE`; the `.md` extension is optional for Markdown files. Install or refresh one provider with `cargo bake agent:context:install --package CRATE`.

Run `cargo bake agent:context:install` to install or refresh context from all resolved dependencies. Files are copied under `.agents/context/SELECTOR/`, where the selector is the crate name or the displayed `crate@version` selector. Discover installed Markdown files with `rg --files --hidden --no-ignore .agents/context -g "*.md"`, then read the guides relevant to the task. Install missing guidance and refresh it after changing dependencies.

The `.agents/context/` directory contains installed dependency copies. In a Git checkout, the install task maintains a marked block in `.git/info/exclude` for that directory and exact dependency-installed skill directories. Project-owned skill directories remain trackable. Repository instructions can direct agents to discover and read the installed guides. Edit provider sources and reinstall to update dependency copies.

## Provide context from a crate

Create a top-level `context/` directory in the crate and add Markdown guides that help users and coding agents complete practical tasks with the crate. For example:

```text
my-crate/
├── Cargo.toml
├── context/
│   ├── getting-started.md
│   ├── configuration.md
│   └── troubleshooting.md
└── src/
```

When `Cargo.toml` has an explicit `include` list, include `context/**` so the guides are present in the published crate.

Write focused, actionable guides with descriptive filenames, a clear heading, and a useful introduction. Link related guides so agents can follow them when they need more detail.

## Provide a skill from context

A context document can also be distributed as an Agent Skill. Put the Markdown document directly in `context/` and give it YAML front matter with `metadata.type: skill` and a description that says what the skill does and when to use it:

```text
my-crate/
├── context/
│   ├── initial-gem-setup.md
│   └── initial-gem-setup/
│       └── references/
│           └── project-checklist.md
└── src/
```

```markdown
---
metadata:
  type: skill
description: Set up a new Ruby gem using the project conventions.
---

# Initial Gem Setup

Follow the setup steps and use the bundled references when needed.
```

Bake preserves an authored `name` and uses it for the installed directory. When `name` is absent, Bake derives it from the Cargo package and source filename: `initial-gem-setup.md` in `my-crate` becomes `my-crate-initial-gem-setup/SKILL.md`. Files under the matching `context/initial-gem-setup/` directory are copied alongside it as skill resources. The installer retains the authored description, additional metadata, and original Markdown body, and adds resolved provenance under `metadata`. Skill documents and their resources install under `.agents/skills/`. In this crate, `context/usage.md` defaults to `bake-agent-context-usage/SKILL.md`.

The required skill front matter fields are `metadata.type: skill` and a non-empty `description`. Other types install as ordinary context. Additional metadata is preserved in the generated skill. Skill resources are copied without parsing their Markdown front matter. There is no portable file-pattern field for skills, so describe the relevant tasks and files in `description`.

Run `cargo bake agent:context:install` to install ordinary context and all discovered skills. Use `--package CRATE` to install both from one provider. For independent skill management, `cargo bake agent:context:skill:install` installs all skills by default; `--package CRATE` selects one provider, `--skill NAME` selects one uniquely named skill using its installed name, and both select a skill from a specific crate. Run `cargo bake agent:context:skill:list` to discover available skills.

Installed skills live under `.agents/skills/`. Bake records ownership there so it can update dependency-provided skills without overwriting project-owned skills. The installer adds exact entries to `.git/info/exclude` for dependency-owned skill directories; it does not ignore the whole `.agents/skills/` directory. Skill names are global within that directory. Authored names are preserved; missing names default to `<crate>-<filename>`. If multiple versions of one crate provide the same skill, select one provider explicitly. The top-level resource name `SKILL.md` is reserved for generated instructions.

Read the installed Markdown guides under `.agents/context/SELECTOR/` for guidance from each provider, including the language-agnostic specification and recommendations for organizing package context and repository instructions.

The provider's `context/` directory is source content and should be versioned with the crate. It is different from `.agents/context/`, which is a generated copy installed into a consumer project.

Installed `SKILL.md` front matter records `metadata.type: skill` and the resolved `metadata.ecosystem`, `metadata.package`, and `metadata.version`. Cargo refreshes reconcile skills within their selected ecosystem and package scope. A full refresh also removes skills from removed or empty providers. After resolving an installation error, rerun the command to refresh generated files.
