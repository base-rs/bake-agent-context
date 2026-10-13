// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use bake::Registry;
use bake_agent_context::{Definition, Installer, Package, SkillDefinition};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const INSTALLED_SKILL_NAME: &str = "docs-provider-initial-gem-setup";

fn write(root: &Path, relative_path: &str, contents: &str) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn skill_definitions(
    installer: &Installer,
    selector: Option<&str>,
) -> bake::Result<Vec<SkillDefinition>> {
    let packages = if let Some(selector) = selector {
        let Some(provider) = installer.find_provider(selector)? else {
            return Ok(Vec::new());
        };
        vec![provider]
    } else {
        installer.providers().to_vec()
    };

    let mut skills = Vec::new();
    for provider in packages {
        for definition in Package::load(provider)?.definitions {
            if let Definition::Skill(skill) = definition {
                skills.push(skill);
            }
        }
    }
    Ok(skills)
}

fn install_context_and_skills(
    installer: &Installer,
    selector: Option<&str>,
) -> bake::Result<Vec<String>> {
    Ok(installer.install(selector)?.skills)
}

fn project() -> (TempDir, std::path::PathBuf) {
    let directory = tempfile::Builder::new()
        .prefix("agent context project ")
        .tempdir()
        .unwrap();
    let root = directory.path().join("consumer");
    let provider = directory.path().join("provider");

    write(
        &root,
        "Cargo.toml",
        "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ndocs-provider = { path = \"../provider\" }\n",
    );
    write(&root, "src/main.rs", "fn main() {}\n");
    write(
        &provider,
        "Cargo.toml",
        "[package]\nname = \"docs-provider\"\nversion = \"1.2.3\"\nedition = \"2024\"\ndescription = \"Guidance from the test provider.\"\n",
    );
    write(&provider, "src/lib.rs", "pub fn provider() {}\n");
    write(
        &provider,
        "context/getting-started.md",
        "# Getting Started\n\nFirst paragraph.\n\nLater details.\n",
    );
    write(
        &provider,
        "context/reference/usage.md",
        "# Usage\n\nNested usage guidance.\n",
    );
    write(&provider, "context/example.json", "{\"enabled\": true}\n");
    write(
        &provider,
        "context/initial-gem-setup.md",
        "---\nmetadata:\n  type: skill\ndescription: Set up a new Ruby gem using the project conventions.\n---\n\n# Initial Gem Setup\n\nCreate the gem files and verify the package.\n",
    );
    write(
        &provider,
        "context/initial-gem-setup/references/checklist.md",
        "Run the project checks before publishing.\n",
    );
    write(
        &provider,
        "context/initial-gem-setup/examples/minimal-project.md",
        "Start with the smallest working project.\n",
    );

    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    (directory, root)
}

#[test]
fn lists_context_definitions_with_skill_markers() {
    let (_directory, root) = project();
    let manifest_path = root.join("Cargo.toml");
    let mut manifest = fs::read_to_string(&manifest_path).unwrap();
    manifest.push_str("\n[workspace]\nmembers = [\"workspace-member\"]\nresolver = \"3\"\n");
    fs::write(manifest_path, manifest).unwrap();
    write(
        &root,
        "workspace-member/Cargo.toml",
        "[package]\nname = \"workspace-member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    );
    write(&root, "workspace-member/src/lib.rs", "pub fn local() {}\n");
    write(
        &root,
        "workspace-member/context/local.md",
        "# Local\n\nThis workspace context should be skipped.\n",
    );

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let installer = Installer::discover(&root).unwrap();

    let providers = installer.providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].selector(), "docs-provider");
    assert_eq!(providers[0].version, "1.2.3");

    let provider = installer.find_provider("docs-provider").unwrap().unwrap();
    let definitions = Package::load(provider).unwrap().definitions;
    let names: Vec<_> = definitions.iter().map(Definition::display_name).collect();
    assert_eq!(
        names,
        [
            "example.json",
            "getting-started.md",
            "initial-gem-setup (skill)",
            "reference/usage.md"
        ]
    );
    assert!(definitions.iter().any(Definition::is_skill));
    assert!(definitions.iter().any(|definition| !definition.is_skill()));
}

#[test]
fn installs_unfamiliar_context_types_alongside_skills() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider/context");
    let guide = "---\nmetadata:\n  type: guide\n---\n\n# Guide\n";
    write(&provider, "guide.md", guide);
    let installer = Installer::discover(&root).unwrap();

    installer.install(None).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".agents/context/docs-provider/guide.md")).unwrap(),
        guide
    );
    assert_eq!(skill_definitions(&installer, None).unwrap().len(), 1);
    assert_eq!(
        install_context_and_skills(&installer, None).unwrap(),
        [format!("{INSTALLED_SKILL_NAME} (docs-provider)")]
    );
    assert!(
        root.join(".agents/skills")
            .join(INSTALLED_SKILL_NAME)
            .join("SKILL.md")
            .exists()
    );
}

#[test]
fn discovers_and_installs_skills_declared_in_yaml_frontmatter() {
    let (_directory, root) = project();
    let installer = Installer::discover(&root).unwrap();
    let skills = skill_definitions(&installer, None).unwrap();

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, INSTALLED_SKILL_NAME);
    assert_eq!(skills[0].package_selector(), "docs-provider");
    assert_eq!(
        skills[0].description,
        "Set up a new Ruby gem using the project conventions."
    );

    assert_eq!(
        install_context_and_skills(&installer, Some("docs-provider")).unwrap(),
        [format!("{INSTALLED_SKILL_NAME} (docs-provider)")]
    );

    let exclude = fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert!(exclude.contains("# BEGIN bake-agent-context\n"));
    assert!(exclude.contains("/.agents/context/\n"));
    assert!(exclude.contains(&format!("/.agents/skills/{INSTALLED_SKILL_NAME}/\n")));
    assert!(exclude.contains("# END bake-agent-context\n"));
    assert!(!exclude.lines().any(|line| line == "/.agents/skills/"));

    let skill_directory = root.join(".agents/skills").join(INSTALLED_SKILL_NAME);
    let skill_markdown = fs::read_to_string(skill_directory.join("SKILL.md")).unwrap();
    assert!(skill_markdown.contains(&format!("name: {INSTALLED_SKILL_NAME}")));
    assert!(skill_markdown.contains("  ecosystem: cargo"));
    assert!(skill_markdown.contains("  package: docs-provider"));
    assert!(skill_markdown.contains("  version: 1.2.3"));
    assert!(skill_markdown.contains("# Initial Gem Setup"));
    assert_eq!(
        fs::read_to_string(skill_directory.join("references/checklist.md")).unwrap(),
        "Run the project checks before publishing.\n"
    );
    assert_eq!(
        fs::read_to_string(skill_directory.join("examples/minimal-project.md")).unwrap(),
        "Start with the smallest working project.\n"
    );

    // Reinstallation updates dependency-owned skills without replacing other skills.
    write(
        &root.parent().unwrap().join("provider"),
        "context/initial-gem-setup/references/checklist.md",
        "Updated checklist.\n",
    );
    install_context_and_skills(&installer, None).unwrap();
    assert_eq!(
        fs::read_to_string(skill_directory.join("references/checklist.md")).unwrap(),
        "Updated checklist.\n"
    );

    fs::remove_file(
        root.parent()
            .unwrap()
            .join("provider/context/initial-gem-setup.md"),
    )
    .unwrap();
    assert!(
        install_context_and_skills(&installer, None)
            .unwrap()
            .is_empty()
    );
    assert!(!skill_directory.exists());
    let exclude = fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert!(!exclude.contains(&format!("/.agents/skills/{INSTALLED_SKILL_NAME}/")));
}

#[test]
fn names_the_usage_skill_from_its_file_and_package() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    write(
        &root,
        "Cargo.toml",
        "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nbake-agent-context = { path = \"../provider\" }\n",
    );
    write(
        &provider,
        "Cargo.toml",
        "[package]\nname = \"bake-agent-context\"\nversion = \"1.2.3\"\nedition = \"2024\"\n",
    );
    write(
        &provider,
        "context/usage.md",
        "---\nmetadata:\n  type: skill\ndescription: Use this skill to install and navigate dependency context.\n---\n\n# Usage\n\nInstall context and read the relevant Markdown guides.\n",
    );

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let installer = Installer::discover(&root).unwrap();
    let skills = skill_definitions(&installer, None).unwrap();
    let usage = skills
        .iter()
        .find(|skill| skill.name == "bake-agent-context-usage")
        .unwrap();

    assert_eq!(
        usage.description,
        "Use this skill to install and navigate dependency context."
    );
}

#[test]
fn skill_installation_does_not_overwrite_project_owned_skills() {
    let (_directory, root) = project();
    write(
        &root,
        &format!(".agents/skills/{INSTALLED_SKILL_NAME}/SKILL.md"),
        "Project-owned skill.\n",
    );
    let installer = Installer::discover(&root).unwrap();

    let error = install_context_and_skills(&installer, None).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("belongs to a project or another provider")
    );
    assert_eq!(
        fs::read_to_string(
            root.join(".agents/skills")
                .join(INSTALLED_SKILL_NAME)
                .join("SKILL.md")
        )
        .unwrap(),
        "Project-owned skill.\n"
    );
}

#[test]
fn maintains_local_excludes_without_hiding_project_owned_skills() {
    let (_directory, root) = project();
    let gitignore = "/target/\n# Project-specific ignore rules.\n";
    write(&root, ".gitignore", gitignore);
    write(
        &root,
        ".agents/skills/local-workflow/SKILL.md",
        "Project-owned skill.\n",
    );
    write(&root, ".git/info/exclude", "# Local user rule\n*.local\n");
    let installer = Installer::discover(&root).unwrap();

    install_context_and_skills(&installer, None).unwrap();

    let actual_gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    let exclude = fs::read_to_string(root.join(".git/info/exclude")).unwrap();
    assert_eq!(actual_gitignore, gitignore);
    assert!(exclude.contains("/.agents/context/\n"));
    assert!(exclude.contains(&format!("/.agents/skills/{INSTALLED_SKILL_NAME}/\n")));
    assert!(exclude.contains("# Local user rule\n*.local\n"));
    assert!(!exclude.lines().any(|line| line == "/.agents/skills/"));
    assert!(!exclude.contains("/.agents/skills/local-workflow/"));
    assert_eq!(
        exclude
            .lines()
            .filter(|line| *line == "# BEGIN bake-agent-context")
            .count(),
        1
    );
    assert_eq!(
        exclude
            .lines()
            .filter(|line| *line == "# END bake-agent-context")
            .count(),
        1
    );

    let generated_skill = Command::new("git")
        .args([
            "check-ignore",
            "--quiet",
            ".agents/skills/docs-provider-initial-gem-setup/SKILL.md",
        ])
        .current_dir(&root)
        .status()
        .unwrap();
    assert!(generated_skill.success());
    let project_skill = Command::new("git")
        .args([
            "check-ignore",
            "--quiet",
            ".agents/skills/local-workflow/SKILL.md",
        ])
        .current_dir(&root)
        .status()
        .unwrap();
    assert_eq!(project_skill.code(), Some(1));

    install_context_and_skills(&installer, None).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".git/info/exclude")).unwrap(),
        exclude
    );
    assert_eq!(
        fs::read_to_string(root.join(".gitignore")).unwrap(),
        gitignore
    );
}

#[test]
fn skill_installation_works_without_a_git_checkout() {
    let (_directory, root) = project();
    fs::remove_dir_all(root.join(".git")).unwrap();
    let installer = Installer::discover(&root).unwrap();

    install_context_and_skills(&installer, None).unwrap();

    assert!(
        root.join(".agents/skills")
            .join(INSTALLED_SKILL_NAME)
            .join("SKILL.md")
            .is_file()
    );
    assert!(!root.join(".gitignore").exists());
}

#[test]
fn installs_context_and_skills() {
    let (_directory, root) = project();
    let output = Registry::discover()
        .unwrap()
        .run_arguments(&root, &["agent:context:install".to_owned()])
        .unwrap();
    assert!(output.contains("Installed context from: docs-provider"));
    assert!(output.contains(&format!(
        "Installed skills: {INSTALLED_SKILL_NAME} (docs-provider)"
    )));

    let installed = root.join(".agents/context/docs-provider");
    assert_eq!(
        fs::read_to_string(installed.join("reference/usage.md")).unwrap(),
        "# Usage\n\nNested usage guidance.\n"
    );
    assert!(
        root.join(".agents/skills")
            .join(INSTALLED_SKILL_NAME)
            .join("SKILL.md")
            .is_file()
    );
}

#[test]
fn copies_ordinary_document_metadata_without_interpreting_descriptions() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    let guide = "---\ndescription: {text: Provider metadata.}\n---\n\n# Guide\n";
    write(&provider, "context/guide.md", guide);
    let installer = Installer::discover(&root).unwrap();

    assert_eq!(
        installer
            .show_context_file("docs-provider", "guide")
            .unwrap()
            .as_deref(),
        Some(guide)
    );
    installer.install(None).unwrap();
    assert_eq!(
        fs::read_to_string(root.join(".agents/context/docs-provider/guide.md")).unwrap(),
        guide
    );
}

#[test]
fn context_only_installation_updates_git_exclusions() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider/context");
    fs::remove_file(provider.join("initial-gem-setup.md")).unwrap();
    fs::remove_dir_all(provider.join("initial-gem-setup")).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );

    let output = Registry::discover()
        .unwrap()
        .run_arguments(&root, &["agent:context:install".to_owned()])
        .unwrap();
    assert!(output.contains("Installed context from: docs-provider"));
    let path = ".agents/context/docs-provider/getting-started.md";
    assert!(root.join(path).is_file());
    assert!(
        Command::new("git")
            .args(["check-ignore", "--quiet", path])
            .current_dir(&root)
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn installs_documents_without_headings_and_omits_empty_directories() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    write(
        &provider,
        "context/no-heading.md",
        "Words without punctuation\n",
    );
    fs::create_dir_all(provider.join("context/empty-directory")).unwrap();

    let installer = Installer::discover(&root).unwrap();
    assert_eq!(installer.context_path(), root.join(".agents/context"));
    assert_eq!(installer.install(None).unwrap().context, ["docs-provider"]);
    assert!(
        !root
            .join(".agents/context/docs-provider/empty-directory")
            .exists()
    );

    assert_eq!(
        fs::read_to_string(root.join(".agents/context/docs-provider/no-heading.md")).unwrap(),
        "Words without punctuation\n"
    );
}

#[test]
fn discovers_multiple_context_providers_and_orders_their_skills() {
    let (_directory, root) = project();
    let second_provider = root.parent().unwrap().join("second-provider");
    write(
        &second_provider,
        "Cargo.toml",
        "[package]\nname = \"second-provider\"\nversion = \"2.0.0\"\nedition = \"2024\"\ndescription = \"A second context provider.\"\n",
    );
    write(&second_provider, "src/lib.rs", "pub fn provider() {}\n");
    write(
        &second_provider,
        "context/guide.md",
        "# Second Provider\n\nUse the second provider guide.\n",
    );
    write(
        &second_provider,
        "context/release.md",
        "---\nmetadata:\n  type: skill\ndescription: Prepare a release.\n---\n\n# Release\n\nReview the release notes first.\n",
    );
    write(
        &root,
        "Cargo.toml",
        "[package]\nname = \"consumer\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ndocs-provider = { path = \"../provider\" }\nsecond-provider = { path = \"../second-provider\" }\n",
    );

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let installer = Installer::discover(&root).unwrap();
    assert_eq!(
        installer
            .providers()
            .iter()
            .map(|package| package.name.as_str())
            .collect::<Vec<_>>(),
        ["docs-provider", "second-provider"]
    );
    assert_eq!(
        installer.install(None).unwrap().context,
        ["docs-provider", "second-provider"]
    );

    let skills = skill_definitions(&installer, None).unwrap();
    assert_eq!(
        skills
            .iter()
            .map(|skill| skill.name.as_str())
            .collect::<Vec<_>>(),
        ["docs-provider-initial-gem-setup", "second-provider-release"]
    );
    assert_eq!(
        install_context_and_skills(&installer, None).unwrap(),
        [
            "docs-provider-initial-gem-setup (docs-provider)",
            "second-provider-release (second-provider)"
        ]
    );
}

#[test]
fn sorts_skills_within_a_provider_by_name() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    write(
        &provider,
        "context/another-skill.md",
        "---\nmetadata:\n  type: skill\ndescription: A second test skill.\n---\n\n# Another Skill\n",
    );

    let installer = Installer::discover(&root).unwrap();
    let skills = skill_definitions(&installer, None).unwrap();
    let names: Vec<_> = skills.iter().map(|skill| skill.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "docs-provider-another-skill",
            "docs-provider-initial-gem-setup"
        ]
    );
}

#[test]
fn installing_one_provider_refreshes_all_of_its_skills() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    write(
        &provider,
        "context/secondary.md",
        "---\nmetadata:\n  type: skill\ndescription: A secondary skill.\n---\n\n# Secondary\n",
    );
    let installer = Installer::discover(&root).unwrap();

    install_context_and_skills(&installer, None).unwrap();
    assert_eq!(
        installer.install(Some("docs-provider")).unwrap().skills,
        [
            format!("{INSTALLED_SKILL_NAME} (docs-provider)"),
            "docs-provider-secondary (docs-provider)".to_owned()
        ]
    );
    fs::remove_file(provider.join("context/secondary.md")).unwrap();
    assert_eq!(
        installer.install(None).unwrap().skills,
        [format!("{INSTALLED_SKILL_NAME} (docs-provider)")]
    );

    let skills_root = root.join(".agents/skills");
    assert!(!skills_root.join("docs-provider-secondary").exists());
    assert!(skills_root.join(INSTALLED_SKILL_NAME).is_dir());
}

#[cfg(unix)]
#[test]
fn reports_ownership_read_and_skill_directory_creation_errors() {
    let (_directory, root) = project();
    let ownership_path = root.join(".agents/skills/provider-workflow/SKILL.md");
    write(
        &root,
        ".agents/skills/provider-workflow/SKILL.md",
        "---\nmetadata:\n  type: skill\n  ecosystem: cargo\n  package: provider\n  version: '1'\n---\n",
    );
    fs::set_permissions(&ownership_path, fs::Permissions::from_mode(0o000)).unwrap();
    let installer = Installer::discover(&root).unwrap();
    let error = install_context_and_skills(&installer, None)
        .unwrap_err()
        .to_string();
    fs::set_permissions(&ownership_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(error.contains("cannot read"), "{error}");

    let (_directory, root) = project();
    let agents_directory = root.join(".agents");
    fs::create_dir_all(&agents_directory).unwrap();
    fs::set_permissions(&agents_directory, fs::Permissions::from_mode(0o555)).unwrap();
    let installer = Installer::discover(&root).unwrap();
    let error = install_context_and_skills(&installer, None)
        .unwrap_err()
        .to_string();
    fs::set_permissions(&agents_directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(error.contains("cannot create"), "{error}");
}

#[cfg(unix)]
#[test]
fn reports_context_removal_errors_without_overwriting_existing_files() {
    let (_directory, root) = project();
    let context_root = root.join(".agents/context");
    write(
        &root,
        ".agents/context/docs-provider/old.md",
        "Old content.\n",
    );
    fs::set_permissions(&context_root, fs::Permissions::from_mode(0o555)).unwrap();

    let installer = Installer::discover(&root).unwrap();
    let result = installer.install(Some("docs-provider"));
    fs::set_permissions(&context_root, fs::Permissions::from_mode(0o755)).unwrap();

    let error = result.unwrap_err().to_string();
    assert!(error.contains("cannot remove"), "{error}");
}

#[test]
fn validates_skill_documents_through_the_dependency_build() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider/context");
    let installer = Installer::discover(&root).unwrap();

    assert_eq!(
        skill_definitions(&installer, Some("docs-provider"))
            .unwrap()
            .len(),
        1
    );
    assert!(
        skill_definitions(&installer, Some("missing-provider"))
            .unwrap()
            .is_empty()
    );
    assert!(
        install_context_and_skills(&installer, Some("missing-provider"))
            .unwrap_err()
            .to_string()
            .contains("no context found")
    );

    write(
        &provider,
        "another-skill.md",
        "---\nmetadata:\n  type: skill\ndescription: Another skill.\n---\n\n# Another Skill\n",
    );
    assert_eq!(
        skill_definitions(&installer, Some("docs-provider"))
            .unwrap()
            .len(),
        2
    );

    for (name, contents, message) in [
        (
            "invalid-yaml.md",
            "---\ntype: [skill\n---\n\n# Skill\n",
            "invalid YAML front matter",
        ),
        (
            "Bad_Name.md",
            "---\nmetadata:\n  type: skill\ndescription: Invalid name.\n---\n\n# Skill\n",
            "invalid skill name",
        ),
        (
            "missing-description.md",
            "---\nmetadata:\n  type: skill\n---\n\n# Skill\n",
            "requires a description",
        ),
        (
            "long-description.md",
            &format!(
                "---\nmetadata:\n  type: skill\ndescription: {}\n---\n\n# Skill\n",
                "x".repeat(1025)
            ),
            "1024 characters",
        ),
    ] {
        write(&provider, name, contents);
        assert!(
            skill_definitions(&installer, None)
                .unwrap_err()
                .to_string()
                .contains(message),
            "{name}"
        );
        assert!(
            install_context_and_skills(&installer, Some("docs-provider"))
                .unwrap_err()
                .to_string()
                .contains(message),
            "{name}"
        );
        fs::remove_file(provider.join(name)).unwrap();
    }

    write(
        &provider,
        "nested/skill.md",
        "---\nmetadata:\n  type: skill\ndescription: Nested.\n---\n\n# Skill\n",
    );
    assert!(
        skill_definitions(&installer, None)
            .unwrap_err()
            .to_string()
            .contains("directly inside context")
    );
    assert!(
        install_context_and_skills(&installer, Some("docs-provider"))
            .unwrap_err()
            .to_string()
            .contains("directly inside context")
    );
    fs::remove_dir_all(provider.join("nested")).unwrap();

    write(
        &provider,
        "blocked.md",
        "---\nmetadata:\n  type: skill\ndescription: Blocked.\n---\n\n# Blocked\n",
    );
    write(&provider, "blocked", "not a directory");
    assert!(
        skill_definitions(&installer, None)
            .unwrap_err()
            .to_string()
            .contains("is not a directory")
    );
    assert!(
        install_context_and_skills(&installer, Some("docs-provider"))
            .unwrap_err()
            .to_string()
            .contains("is not a directory")
    );
}

#[cfg(unix)]
#[test]
fn rejects_non_regular_skill_assets_in_the_dependency_build() {
    use std::os::unix::fs::symlink;
    use std::process::Command;

    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    let assets = provider.join("context/initial-gem-setup");
    let target = root.join("asset-target.txt");
    fs::write(&target, "target").unwrap();
    symlink(&target, assets.join("link")).unwrap();
    let error = install_context_and_skills(&Installer::discover(&root).unwrap(), None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot contain symbolic links"), "{error}");

    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    fs::write(
        provider.join("context/initial-gem-setup/SKILL.md"),
        "Reserved skill asset.\n",
    )
    .unwrap();
    let error = install_context_and_skills(&Installer::discover(&root).unwrap(), None)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("reserved for generated skill files"),
        "{error}"
    );

    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    let socket_path = provider.join("context/initial-gem-setup/socket");
    let status = Command::new("mkfifo").arg(&socket_path).status().unwrap();
    assert!(status.success(), "mkfifo failed: {status}");
    let error = install_context_and_skills(&Installer::discover(&root).unwrap(), None)
        .unwrap_err()
        .to_string();
    assert!(error.contains("unsupported skill asset"), "{error}");
}

#[test]
fn preserves_the_original_body_without_a_final_newline() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider");
    write(
        &provider,
        "context/no-final-newline.md",
        "---\nmetadata:\n  type: skill\ndescription: A skill without a final newline.\n---\n\n# No Final Newline",
    );

    install_context_and_skills(&Installer::discover(&root).unwrap(), None).unwrap();
    let installed =
        fs::read_to_string(root.join(".agents/skills/docs-provider-no-final-newline/SKILL.md"))
            .unwrap();
    assert!(installed.ends_with("# No Final Newline"));
}

#[test]
fn reports_public_installer_and_cargo_metadata_errors() {
    let (_directory, root) = project();
    let installer = Installer::discover(&root).unwrap();
    fs::create_dir_all(root.join(".agents")).unwrap();
    let blocked = root.join(".agents/context");
    fs::write(&blocked, "not a directory").unwrap();
    let error = installer
        .install(Some("docs-provider"))
        .unwrap_err()
        .to_string();
    assert!(error.contains(&blocked.display().to_string()), "{error}");
    assert_eq!(fs::read_to_string(blocked).unwrap(), "not a directory");

    let directory = tempfile::tempdir().unwrap();
    write(
        directory.path(),
        "Cargo.toml",
        "this is not valid TOML = [\n",
    );
    let error = Installer::discover(directory.path()).unwrap_err();
    assert!(error.to_string().contains("cargo metadata failed"));
}

#[test]
fn rejects_invalid_skill_ownership_and_installation_paths() {
    let (_directory, root) = project();
    let skills_root = root.join(".agents/skills");
    fs::create_dir_all(root.join(".agents")).unwrap();
    fs::write(&skills_root, "not a directory").unwrap();
    let installer = Installer::discover(&root).unwrap();
    assert!(
        install_context_and_skills(&installer, None)
            .unwrap_err()
            .to_string()
            .contains("not a regular directory")
    );

    let (_directory, root) = project();
    let installer = Installer::discover(&root).unwrap();
    let ownership_path = root.join(".agents/skills/provider-workflow/SKILL.md");
    write(
        &root,
        ".agents/skills/provider-workflow/SKILL.md",
        "---\nmetadata: [invalid\n---\n",
    );
    assert!(
        install_context_and_skills(&installer, None)
            .unwrap_err()
            .to_string()
            .contains("invalid YAML front matter")
    );

    fs::write(
        &ownership_path,
        "---\nmetadata:\n  type: skill\n  ecosystem: cargo\n  package: ' '\n  version: '1'\n---\n",
    )
    .unwrap();
    assert!(
        install_context_and_skills(&installer, None)
            .unwrap_err()
            .to_string()
            .contains("invalid skill ownership")
    );
}

#[test]
fn show_supports_implicit_markdown_extension_and_rejects_parent_paths() {
    let (_directory, root) = project();
    let installer = Installer::discover(&root).unwrap();

    assert_eq!(
        installer
            .show_context_file("docs-provider", "getting-started")
            .unwrap()
            .unwrap(),
        "# Getting Started\n\nFirst paragraph.\n\nLater details.\n"
    );
    assert!(
        installer
            .show_context_file("docs-provider", "../Cargo.toml")
            .is_err()
    );
}

#[test]
fn registers_the_context_task_names() {
    let registry = Registry::discover().unwrap();
    let names: Vec<_> = registry.tasks().map(|task| task.name()).collect();

    assert!(names.contains(&"agent:context:list"));
    assert!(names.contains(&"agent:context:show"));
    assert!(names.contains(&"agent:context:install"));
}

#[test]
fn bake_tasks_cover_context_and_skill_lookup_and_installation() {
    let (_directory, root) = project();
    let registry = Registry::discover().unwrap();

    let output = registry
        .run_arguments(&root, &["agent:context:list".to_owned()])
        .unwrap();
    assert!(output.contains("Crates with context available:"));

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:list".to_owned(),
                "--package".to_owned(),
                "missing".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("No context found for crate 'missing'"));

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:list".to_owned(),
                "--package".to_owned(),
                "docs-provider".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("getting-started.md"));
    assert!(output.contains("initial-gem-setup (skill)"));

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:show".to_owned(),
                "--package".to_owned(),
                "docs-provider".to_owned(),
                "--file".to_owned(),
                "getting-started".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("First paragraph."));
    assert!(
        Registry::discover()
            .unwrap()
            .run_arguments(
                &root,
                &[
                    "agent:context:show".to_owned(),
                    "--package".to_owned(),
                    "docs-provider".to_owned(),
                    "--file".to_owned(),
                    "absent".to_owned(),
                ],
            )
            .is_err()
    );

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:show".to_owned(),
                "--package".to_owned(),
                "docs-provider".to_owned(),
                "--file".to_owned(),
                "initial-gem-setup".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("Create the gem files and verify the package."));

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:install".to_owned(),
                "--package".to_owned(),
                "docs-provider".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("Installed context from: docs-provider"));
    assert!(output.contains("Installed skills: docs-provider-initial-gem-setup"));
}

#[test]
fn bake_tasks_report_empty_context_and_skill_sets() {
    let (_directory, root) = project();
    fs::remove_dir_all(root.parent().unwrap().join("provider/context")).unwrap();

    assert_eq!(
        Registry::discover()
            .unwrap()
            .run_arguments(&root, &["agent:context:list".to_owned()])
            .unwrap()
            .trim_end(),
        "No dependency context files found"
    );
    assert_eq!(
        Registry::discover()
            .unwrap()
            .run_arguments(&root, &["agent:context:install".to_owned()])
            .unwrap()
            .trim_end(),
        "No dependency context or skills were installed"
    );
}

#[test]
fn tasks_distinguish_skill_only_dependencies_from_missing_context() {
    let (_directory, root) = project();
    let provider = root.parent().unwrap().join("provider/context");
    fs::remove_file(provider.join("getting-started.md")).unwrap();
    fs::remove_file(provider.join("example.json")).unwrap();
    fs::remove_dir_all(provider.join("reference")).unwrap();

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:list".to_owned(),
                "--package".to_owned(),
                "docs-provider".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("initial-gem-setup (skill)"));

    let output = Registry::discover()
        .unwrap()
        .run_arguments(
            &root,
            &[
                "agent:context:install".to_owned(),
                "--package".to_owned(),
                "docs-provider".to_owned(),
            ],
        )
        .unwrap();
    assert!(output.contains("Installed skills: docs-provider-initial-gem-setup"));
    assert!(!output.contains("Installed context from:"));
}
