// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::document::ContextDocument;
use super::installer::{ContextPackage, remove_existing};
use super::ownership::installed_skill_owners;
#[cfg(test)]
use super::test_filesystem as filesystem;
use bake::{Error, Result};
use serde_yaml_ng::Value;
use std::collections::HashSet;
#[cfg(not(test))]
use std::fs as filesystem;
use std::path::{Path, PathBuf};

/// A skill declared by a dependency context document.
#[derive(Clone, Debug)]
pub struct Skill {
    /// Authored or derived name used for the installed directory.
    pub name: String,
    pub description: String,
    pub package: ContextPackage,
    pub(crate) source_name: String,
    assets: Option<PathBuf>,
    content: String,
}

impl Skill {
    /// The package selector accepted by `--package`.
    pub fn package_selector(&self) -> &str {
        self.package.selector()
    }

    /// Installed instructions, including provider metadata.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Apply skill-specific rules to a parsed document.
    pub(crate) fn from_document(
        package: &ContextPackage,
        document: &ContextDocument,
    ) -> Result<Self> {
        let source = &document.path;
        if source.parent() != Some(package.context_path.as_path()) {
            return Err(Error::new(format!(
                "skill document {} must be directly inside context/",
                source.display()
            )));
        }

        // Preserve authored names and derive missing names from the provider and file:
        let source_name = source_name(source)?;
        let name = match document.frontmatter.get("name") {
            Some(Value::String(name)) => name.clone(),
            None => format!(
                "{}-{source_name}",
                package.name.to_ascii_lowercase().replace('_', "-")
            ),
            _ => {
                return Err(Error::new(format!(
                    "invalid skill name in {}",
                    source.display()
                )));
            }
        };
        validate_skill_name(&name)?;
        let description = document
            .frontmatter
            .get("description")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty() && value.trim().chars().count() <= 1024)
            .ok_or_else(|| {
                Error::new(format!(
                    "skill {name:?} in {} requires a description of 1–1024 characters",
                    source.display()
                ))
            })?
            .to_owned();

        // Add resolved provenance while retaining the provider's remaining metadata:
        let assets = skill_assets_path(&package.context_path, &source_name)?;
        let mut frontmatter = document.frontmatter.clone();
        frontmatter.insert("name".into(), name.clone().into());
        let mut metadata = document.metadata().cloned().unwrap_or_default();
        metadata.insert("ecosystem".into(), "cargo".into());
        metadata.insert("package".into(), package.name.clone().into());
        metadata.insert("version".into(), package.version.clone().into());
        frontmatter.insert("metadata".into(), Value::Mapping(metadata));
        let content = document.render(frontmatter);

        Ok(Self {
            name,
            description,
            package: package.clone(),
            source_name,
            assets,
            content,
        })
    }

    /// Write instructions before resources so ownership remains identifiable during refresh.
    fn write(&self, destination: &Path) -> Result<()> {
        filesystem::create_dir_all(destination).map_err(|error| {
            Error::new(format!("cannot create {}: {error}", destination.display()))
        })?;
        let path = destination.join("SKILL.md");
        if let Err(error) = filesystem::write(&path, &self.content) {
            // A failed instruction write has not established usable ownership:
            let _ = filesystem::remove_dir_all(destination);
            return Err(Error::new(format!(
                "cannot write {}: {error}",
                path.display()
            )));
        }
        if let Some(assets) = &self.assets {
            copy_skill_assets(assets, destination, true)?;
        }
        Ok(())
    }
}

fn skill_assets_path(context_path: &Path, source_name: &str) -> Result<Option<PathBuf>> {
    let assets = context_path.join(source_name);
    match filesystem::symlink_metadata(&assets) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(Some(assets)),
        Ok(_) => Err(Error::new(format!(
            "skill assets path {} is not a directory",
            assets.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::new(format!(
            "cannot inspect skill assets {}: {error}",
            assets.display()
        ))),
    }
}

/// Refresh loaded skills and reconcile stale destinations within the selected scope.
pub(crate) fn install_definitions(
    root: &Path,
    skills: &[Skill],
    package: Option<&str>,
    reconcile: bool,
) -> Result<Vec<String>> {
    // Check selected names and destination ownership before refreshing directories:
    let mut names = HashSet::new();
    for skill in skills {
        if !names.insert(skill.name.clone()) {
            return Err(Error::new(format!(
                "multiple selected crates provide skill {:?}; select one with --package",
                skill.name
            )));
        }
    }
    let skills_root = root.join(".agents/skills");
    let owners = installed_skill_owners(&skills_root)?;
    for skill in skills {
        let destination = skills_root.join(&skill.name);
        if path_exists(&destination)?
            && owners.get(&skill.name).is_none_or(|owner| {
                owner.ecosystem != "cargo" || owner.package != skill.package.name
            })
        {
            return Err(Error::new(format!(
                "skill destination {} belongs to a project or another provider",
                destination.display()
            )));
        }
    }
    let stale: Vec<_> = owners
        .iter()
        .filter(|(name, owner)| {
            reconcile
                && owner.ecosystem == "cargo"
                && package.is_none_or(|package| owner.package == package)
                && !names.contains(*name)
        })
        .map(|(name, _)| name.clone())
        .collect();

    // Write each selected skill from the instructions already loaded:
    filesystem::create_dir_all(&skills_root)
        .map_err(|error| Error::new(format!("cannot create {}: {error}", skills_root.display())))?;
    for skill in skills {
        let destination = skills_root.join(&skill.name);
        remove_existing(&destination)?;
        skill.write(&destination)?;
    }

    // Remove stale skills and update this project's local Git exclusions:
    for name in &stale {
        remove_existing(&skills_root.join(name))?;
    }
    let remaining = owners
        .keys()
        .filter(|name| !stale.contains(name))
        .cloned()
        .chain(names);
    if let Some(update) = super::exclude::prepare(root, remaining)? {
        update.apply()?;
    }
    Ok(skills
        .iter()
        .map(|skill| format!("{} ({})", skill.name, skill.package_selector()))
        .collect())
}

pub(crate) fn validate_skill_name(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(Error::new(format!(
            "invalid skill name {name:?}; use 1–64 lowercase ASCII letters, digits, or single hyphens"
        )))
    }
}

fn source_name(source: &Path) -> Result<String> {
    let Some(stem) = source.file_stem().and_then(|stem| stem.to_str()) else {
        return Err(Error::new(format!(
            "invalid skill filename: {}",
            source.display()
        )));
    };
    Ok(stem.to_owned())
}

fn copy_skill_assets(source: &Path, destination: &Path, top_level: bool) -> Result<()> {
    let entries = filesystem::read_dir(source)
        .map_err(|error| Error::new(format!("cannot read {}: {error}", source.display())))?;
    let mut entries = entries.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        copy_skill_asset(&source_path, &destination_path, top_level)?;
    }

    Ok(())
}

fn copy_skill_asset(source: &Path, destination: &Path, top_level: bool) -> Result<()> {
    let metadata = inspect_skill_asset(source)?;
    let file_type = metadata.file_type();

    let name = source.file_name().unwrap_or_default().to_string_lossy();
    if top_level && name.eq_ignore_ascii_case("SKILL.md") {
        return Err(Error::new(format!(
            "{} is reserved for generated skill files",
            source.display()
        )));
    }

    if file_type.is_symlink() {
        return Err(Error::new(format!(
            "skill assets cannot contain symbolic links: {}",
            source.display()
        )));
    } else if file_type.is_dir() {
        filesystem::create_dir(destination).map_err(|error| {
            Error::new(format!("cannot create {}: {error}", destination.display()))
        })?;
        copy_skill_assets(source, destination, false)?;
    } else if file_type.is_file() {
        filesystem::copy(source, destination)
            .map(|_| ())
            .map_err(|error| {
                Error::new(format!(
                    "cannot copy {} to {}: {error}",
                    source.display(),
                    destination.display()
                ))
            })?;
    } else {
        return Err(Error::new(format!(
            "unsupported skill asset: {}",
            source.display()
        )));
    }
    Ok(())
}

fn inspect_skill_asset(path: &Path) -> Result<filesystem::Metadata> {
    filesystem::symlink_metadata(path)
        .map_err(|error| Error::new(format!("cannot inspect {}: {error}", path.display())))
}

fn path_exists(path: &Path) -> Result<bool> {
    match filesystem::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::new(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::super::installer::{Definition, Installer};
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn list_skills(installer: &Installer, selector: Option<&str>) -> Result<Vec<Skill>> {
        let packages = if let Some(selector) = selector {
            let Some(package) = installer.find_package(selector)? else {
                return Ok(Vec::new());
            };
            vec![package]
        } else {
            installer.packages().to_vec()
        };

        let mut skills = Vec::new();
        for package in packages {
            for definition in
                super::super::package_context::PackageContext::load(&package)?.definitions
            {
                if let Definition::Skill(skill) = definition {
                    skills.push(skill);
                }
            }
        }
        skills.sort_by(|left, right| {
            left.package
                .name
                .cmp(&right.package.name)
                .then_with(|| left.package.version.cmp(&right.package.version))
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(skills)
    }

    fn install_skills(installer: &Installer, selector: Option<&str>) -> Result<Vec<String>> {
        let skills = list_skills(installer, selector)?;
        let package = if selector.is_some() {
            skills.first().map(|skill| skill.package.name.as_str())
        } else {
            None
        };
        install_definitions(installer.root(), &skills, package, true)
    }

    fn provider(root: &Path, name: &str, version: &str) -> ContextPackage {
        let path = root.join(format!("{name}-{version}/context"));
        fs::create_dir_all(&path).unwrap();
        ContextPackage::for_test(name, version, path)
    }

    fn write_skill(package: &ContextPackage, name: &str, fields: &str, body: &str) {
        fs::write(
            package.context_path.join(format!("{name}.md")),
            format!("---\nmetadata:\n  type: skill\n{fields}---\n{body}"),
        )
        .unwrap();
    }

    fn seed_owner(path: &Path, ecosystem: &str, package: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("SKILL.md"), format!("---\nmetadata:\n  type: skill\n  ecosystem: {ecosystem}\n  package: {package}\n  version: '1.0'\n---\nInstructions")).unwrap();
    }

    #[test]
    fn preserves_authored_names_descriptions_and_metadata() {
        let directory = tempdir().unwrap();
        let package = provider(directory.path(), "provider", "2.0.0");
        fs::write(
            package.context_path.join("workflow.md"),
            include_str!("../tests/fixtures/SKILL.md"),
        )
        .unwrap();
        let installer = Installer::for_test(directory.path(), vec![package.clone()]);
        let skills = list_skills(&installer, None).unwrap();
        assert_eq!(skills[0].name, "provider-workflow");
        assert!(skills[0].content().contains("author: Provider"));
        assert!(skills[0].content().contains("version: 2.0.0"));
        write_skill(
            &package,
            "workflow",
            "name: custom-workflow\ndescription: '  Run workflow.  '\n",
            "\nBody",
        );
        let skills = list_skills(&installer, None).unwrap();
        assert_eq!(skills[0].name, "custom-workflow");
        assert_eq!(skills[0].description, "  Run workflow.  ");
        install_skills(&installer, None).unwrap();
        assert!(
            directory
                .path()
                .join(".agents/skills/custom-workflow/SKILL.md")
                .is_file()
        );
    }

    #[test]
    fn sorts_providers_and_detects_ambiguous_installed_names() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let first = provider(root, "a_provider", "1.0.0");
        let second = provider(root, "a_provider", "2.0.0");
        let last = provider(root, "z-provider", "1.0.0");
        fs::write(first.context_path.join("guide.md"), "# Guide\n").unwrap();
        for package in [&last, &second, &first] {
            write_skill(package, "zeta", "description: Zeta.\n", "Z");
            write_skill(package, "alpha", "description: Alpha.\n", "A");
        }
        let installer = Installer::for_test(root, vec![last, second, first]);
        assert!(
            list_skills(&installer, Some("missing-provider"))
                .unwrap()
                .is_empty()
        );
        let skills = list_skills(&installer, None).unwrap();
        assert_eq!(skills[0].name, "a-provider-alpha");
        assert_eq!(skills[0].package.version, "1.0.0");
        assert_eq!(skills.last().unwrap().name, "z-provider-zeta");
        assert!(
            install_skills(&installer, None)
                .unwrap_err()
                .to_string()
                .contains("multiple selected")
        );
        assert!(install_skills(&installer, Some("a_provider")).is_err());
        assert!(list_skills(&installer, Some("a_provider")).is_err());
    }

    #[test]
    fn validates_names_and_description_boundaries() {
        let directory = tempdir().unwrap();
        let package = provider(directory.path(), "provider", "1.0.0");
        let installer = Installer::for_test(directory.path(), vec![package.clone()]);
        for fields in [
            "name: 123\ndescription: Valid.\n".to_owned(),
            "name: null\ndescription: Valid.\n".to_owned(),
            "name: Bad_Name\ndescription: Valid.\n".to_owned(),
            "description: []\n".to_owned(),
            "description: '   '\n".to_owned(),
            format!("description: {}\n", "é".repeat(1025)),
        ] {
            write_skill(&package, "workflow", &fields, "Body");
            assert!(list_skills(&installer, None).is_err(), "{fields}");
        }
        write_skill(
            &package,
            "workflow",
            &format!(
                "name: {}\ndescription: '  {}  '\n",
                "a".repeat(64),
                "é".repeat(1024)
            ),
            "Body",
        );
        assert_eq!(list_skills(&installer, None).unwrap()[0].name.len(), 64);
        for name in ["", "-a", "a-", "a--b", "UPPER", &"a".repeat(65)] {
            assert!(validate_skill_name(name).is_err());
        }
        assert!(source_name(Path::new("/")).is_err());
    }

    #[test]
    fn reconciles_only_the_selected_ecosystem_and_package() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let package = provider(root, "provider", "1.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let installer = Installer::for_test(root, vec![package.clone()]);
        let skills = root.join(".agents/skills");
        seed_owner(&skills.join("stale-provider"), "cargo", "provider");
        seed_owner(&skills.join("other-workflow"), "cargo", "other");
        seed_owner(&skills.join("ruby-workflow"), "gem", "provider");
        install_skills(&installer, Some("provider")).unwrap();
        assert!(!skills.join("stale-provider").exists());
        assert!(skills.join("other-workflow").exists());
        assert!(skills.join("ruby-workflow").exists());
        fs::remove_file(package.context_path.join("workflow.md")).unwrap();
        install_skills(&installer, Some("provider")).unwrap();
        assert!(!skills.join("provider-workflow").exists());
        install_skills(&Installer::for_test(root, vec![]), None).unwrap();
        assert!(!skills.join("other-workflow").exists());
        assert!(skills.join("ruby-workflow").exists());
    }

    #[test]
    fn rejects_other_providers_and_accepts_the_same_owner_at_a_new_version() {
        let directory = tempdir().unwrap();
        let package = provider(directory.path(), "provider", "2.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let installer = Installer::for_test(directory.path(), vec![package]);
        let destination = directory.path().join(".agents/skills/provider-workflow");
        for (ecosystem, package) in [("gem", "provider"), ("cargo", "other")] {
            seed_owner(&destination, ecosystem, package);
            assert!(install_skills(&installer, None).is_err());
        }
        seed_owner(&destination, "cargo", "provider");
        install_skills(&installer, None).unwrap();
        assert!(
            fs::read_to_string(destination.join("SKILL.md"))
                .unwrap()
                .contains("version: 2.0.0")
        );
    }

    #[test]
    fn resource_failure_leaves_identifiable_progress_and_retry_refreshes_it() {
        let directory = tempdir().unwrap();
        let package = provider(directory.path(), "provider", "1.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let resources = package.context_path.join("workflow");
        fs::create_dir_all(resources.join("references")).unwrap();
        fs::write(resources.join("references/data.md"), "---\n[invalid YAML\n").unwrap();
        fs::write(resources.join("notes.txt"), "opaque asset").unwrap();
        let installer = Installer::for_test(directory.path(), vec![package.clone()]);
        let failure_path = resources.join("notes.txt");
        let _failure = filesystem::fail_once(filesystem::Operation::Copy, move |path| {
            path == failure_path
        });
        assert!(install_skills(&installer, None).is_err());
        let skills = directory.path().join(".agents/skills");
        assert_eq!(
            installed_skill_owners(&skills).unwrap()["provider-workflow"].package,
            "provider"
        );
        install_skills(&installer, None).unwrap();
        assert_eq!(
            fs::read_to_string(skills.join("provider-workflow/notes.txt")).unwrap(),
            "opaque asset"
        );
        fs::remove_file(resources.join("notes.txt")).unwrap();
        install_skills(&installer, None).unwrap();
        assert!(!skills.join("provider-workflow/notes.txt").exists());
    }

    #[test]
    fn reserves_only_the_top_level_instruction_filename() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("source");
        let destination = directory.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        for name in ["SKILL.md", "skill.MD"] {
            fs::write(source.join(name), "conflict").unwrap();
            assert!(copy_skill_assets(&source, &destination, true).is_err());
            fs::remove_file(source.join(name)).unwrap();
            fs::create_dir(source.join(name)).unwrap();
            assert!(copy_skill_assets(&source, &destination, true).is_err());
            fs::remove_dir(source.join(name)).unwrap();
        }
        fs::create_dir(source.join("references")).unwrap();
        fs::write(source.join("references/SKILL.md"), "opaque").unwrap();
        copy_skill_assets(&source, &destination, true).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("references/SKILL.md")).unwrap(),
            "opaque"
        );
    }
    #[test]
    fn instruction_write_failure_cleans_the_directory_and_retry_succeeds() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let package = provider(root, "provider", "1.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let installer = Installer::for_test(root, vec![package]);
        let destination = root.join(".agents/skills/provider-workflow");
        let instructions = destination.join("SKILL.md");
        let _failure = filesystem::fail_once(filesystem::Operation::Write, move |path| {
            path == instructions
        });
        assert!(
            install_skills(&installer, None)
                .unwrap_err()
                .to_string()
                .contains("cannot write")
        );
        assert!(!destination.exists());
        install_skills(&installer, None).unwrap();
        assert_eq!(
            installed_skill_owners(&root.join(".agents/skills")).unwrap()["provider-workflow"]
                .package,
            "provider"
        );
    }

    #[test]
    fn reports_discovery_and_refresh_io_failures() {
        use filesystem::Operation;
        let directory = tempdir().unwrap();
        let root = directory.path();
        let package = provider(root, "provider", "1.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let installer = Installer::for_test(root, vec![package.clone()]);
        let skills = root.join(".agents/skills");
        let destination = skills.join("provider-workflow");
        for (operation, path) in [
            (Operation::ReadDirectory, package.context_path.clone()),
            (Operation::Inspect, package.context_path.join("workflow")),
            (Operation::Inspect, destination.clone()),
            (Operation::CreateDirectoryTree, destination.clone()),
        ] {
            let _failure = filesystem::fail_once(operation, move |candidate| candidate == path);
            assert!(
                install_skills(&installer, None)
                    .unwrap_err()
                    .to_string()
                    .contains("injected filesystem failure")
            );
        }
        install_skills(&installer, None).unwrap();
        let failure_path = destination.clone();
        let _failure = filesystem::fail_once(Operation::RemoveDirectoryTree, move |path| {
            path == failure_path
        });
        assert!(
            install_skills(&installer, None)
                .unwrap_err()
                .to_string()
                .contains("cannot remove")
        );
        fs::remove_file(package.context_path.join("workflow.md")).unwrap();
        let _failure = filesystem::fail_once(Operation::RemoveDirectoryTree, move |path| {
            path == destination
        });
        assert!(
            install_skills(&installer, None)
                .unwrap_err()
                .to_string()
                .contains("cannot remove")
        );
    }

    #[test]
    fn reports_resource_failures_and_keeps_the_provider_identity() {
        use filesystem::Operation;
        let directory = tempdir().unwrap();
        let root = directory.path();
        let package = provider(root, "provider", "1.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let resources = package.context_path.join("workflow");
        fs::create_dir_all(resources.join("references")).unwrap();
        fs::write(resources.join("references/guide.md"), "# Guide").unwrap();
        let installer = Installer::for_test(root, vec![package]);
        let destination = root.join(".agents/skills/provider-workflow");
        for (operation, path) in [
            (Operation::ReadDirectory, resources.clone()),
            (Operation::ReadDirectoryEntry, resources.clone()),
            (Operation::Inspect, resources.join("references")),
            (Operation::CreateDirectory, destination.join("references")),
            (Operation::ReadDirectory, resources.join("references")),
        ] {
            let _failure = filesystem::fail_once(operation, move |candidate| candidate == path);
            assert!(
                install_skills(&installer, None)
                    .unwrap_err()
                    .to_string()
                    .contains("injected filesystem failure")
            );
            assert_eq!(
                installed_skill_owners(&root.join(".agents/skills")).unwrap()["provider-workflow"]
                    .package,
                "provider"
            );
        }
    }

    #[test]
    fn reports_git_exclusion_errors_after_installing_identifiable_skills() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .arg(root)
                .status()
                .unwrap()
                .success()
        );
        let package = provider(root, "provider", "1.0.0");
        write_skill(&package, "workflow", "description: Workflow.\n", "Body");
        let installer = Installer::for_test(root, vec![package]);
        let exclude = root.join(".git/info/exclude");
        fs::write(&exclude, "# BEGIN bake-agent-context\n").unwrap();
        assert!(install_skills(&installer, None).is_err());
        fs::write(&exclude, "# Project exclusions\n").unwrap();
        let _failure =
            filesystem::fail_once(filesystem::Operation::Write, move |path| path == exclude);
        assert!(
            install_skills(&installer, None)
                .unwrap_err()
                .to_string()
                .contains("injected filesystem failure")
        );
        assert_eq!(
            installed_skill_owners(&root.join(".agents/skills")).unwrap()["provider-workflow"]
                .package,
            "provider"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_skill_filenames() {
        use std::os::unix::ffi::OsStringExt;
        let directory = tempdir().unwrap();
        let package = provider(directory.path(), "provider", "1.0.0");
        let name = std::ffi::OsString::from_vec(b"invalid-\xff.md".to_vec());
        let document = ContextDocument::parse(
            "---\nmetadata: {type: skill}\ndescription: Workflow.\n---\n".to_owned(),
            &package.context_path.join(name),
        )
        .unwrap();
        assert!(
            Skill::from_document(&package, &document)
                .unwrap_err()
                .to_string()
                .contains("invalid skill filename")
        );
    }
}
