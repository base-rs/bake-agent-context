# Agent Context

Keep reusable package guidance separate from repository-only instructions, and make both easy for agents to discover.

## Package guidance

Put guidance that benefits users of a crate in its tracked `context/` directory. Explain package-specific architecture, important invariants, common usage patterns, and decisions that are not obvious from the public API. Keep each guide focused on one topic, use descriptive lowercase filenames, and link related guides together.

Include `context/**` in the published Cargo package when downstream users need the guidance. Agents discover installed Markdown files under `.agents/context/SELECTOR/` or inspect provider files with the list/show commands. Use the `bake-agent-context-usage` skill for file format, skill metadata, and installation details.

Use a skill for a task-oriented procedure that an agent should follow, such as setting up a repository or preparing a pull request. Skills are source Markdown files in `context/` with YAML front matter containing `metadata.type: skill` and a description. Bake installs them under `.agents/skills/` using authored names or deriving missing names from the provider crate and filename. Skill documents are not copied to `.agents/context/`.

## Repository-only instructions

Put guidance that applies only to the current checkout in `.agents/`. This can include local development instructions, project-specific skills, and instructions that should not ship to crate users. Do not put credentials or machine-specific private details in package context.

Repository instructions can direct agents to discover and read relevant Markdown files under `.agents/context/`. `cargo bake agent:context:install` maintains local Git exclusions for installed context and exact dependency-installed skill directories. Project-owned skill directories remain trackable. Edit package skill sources under `context/` and repository-only skills under `.agents/skills/`.

## Install and update

Add `bake-agent-context` to the project's private `bake/` package, directly or through a shared task package such as `socketry-project`. Run `cargo bake agent:context:install` to install ordinary context and skills from resolved dependencies. Run it again after changing a context-providing dependency. Use `--package CRATE` to list, show, or install definitions from one provider.

Before changing a project, follow its agent instructions, read relevant installed Markdown guides under `.agents/context/`, and apply skills that fit the work. For details about the available Bake tasks and their options, use the `bake-agent-context-usage` skill.

Installed `SKILL.md` front matter records `metadata.type: skill` and the resolved `metadata.ecosystem`, `metadata.package`, and `metadata.version`. Cargo refreshes reconcile skills within their selected ecosystem and package scope. A full refresh also removes skills from removed or empty providers. After resolving an installation error, rerun the command to refresh generated files.
