// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::package::Package;
use super::skill::SkillDefinition;
#[cfg(test)]
use super::test_filesystem as fs;
use bake::{Error, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
#[cfg(not(test))]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolved package metadata for a Cargo context provider.
#[derive(Clone, Debug)]
pub struct Specification {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub context_path: PathBuf,
    selector: String,
}

impl Specification {
    /// The name accepted by the task's `--package` option.
    pub fn selector(&self) -> &str {
        &self.selector
    }

    #[cfg(test)]
    pub(super) fn for_test(name: &str, version: &str, context_path: PathBuf) -> Self {
        Self {
            name: name.to_owned(),
            version: version.to_owned(),
            description: Some(format!("{name} documentation")),
            context_path,
            selector: format!("{name}@{version}"),
        }
    }
}

/// An ordinary file loaded from a provider's `context/` directory.
#[derive(Clone, Debug)]
pub struct FileDefinition {
    /// The path relative to the provider's `context/` directory.
    pub path: PathBuf,
    /// The source file path.
    pub source_path: PathBuf,
    pub(crate) document: Option<super::document::Document>,
}

/// One loaded context definition.
#[derive(Clone, Debug)]
pub enum Definition {
    File(FileDefinition),
    Skill(SkillDefinition),
}

impl Definition {
    /// The name shown by `agent:context:list`.
    pub fn display_name(&self) -> String {
        match self {
            Self::File(file) => file
                .path
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/"),
            Self::Skill(skill) => format!(
                "{} (skill)",
                skill.path.file_stem().unwrap().to_string_lossy()
            ),
        }
    }

    /// The source path relative to the provider's context directory.
    pub fn path(&self) -> &Path {
        match self {
            Self::File(file) => &file.path,
            Self::Skill(skill) => &skill.path,
        }
    }

    /// Read the authored content of this definition.
    pub fn content(&self) -> Result<String> {
        match self {
            Self::File(file) => file.content(),
            Self::Skill(skill) => Ok(skill.content().to_owned()),
        }
    }

    /// Whether the definition installs as an Agent Skill.
    pub fn is_skill(&self) -> bool {
        matches!(self, Self::Skill(_))
    }
}

impl FileDefinition {
    /// Read the authored content of the file.
    pub fn content(&self) -> Result<String> {
        if let Some(document) = &self.document {
            return Ok(document.content.clone());
        }

        read_context_file(&self.source_path)
    }
}

/// The provider and skill names refreshed by an installation.
#[derive(Debug)]
pub struct Installation {
    pub context: Vec<String>,
    pub skills: Vec<String>,
}

/// Discovers, lists, reads, and installs context from resolved Cargo dependencies.
#[derive(Clone, Debug)]
pub struct Installer {
    root: PathBuf,
    context_path: PathBuf,
    specifications: Vec<Specification>,
}

impl Installer {
    /// Resolve the project's Cargo packages and find dependencies with `context/` directories.
    pub fn discover(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        let cargo = cargo_executable(env::var_os("CARGO"));
        Self::discover_with_cargo(root, cargo)
    }

    fn discover_with_cargo(root: PathBuf, cargo: OsString) -> Result<Self> {
        let manifest = root.join("Cargo.toml");
        let output = run_cargo_metadata(&root, &cargo)?;
        if !output.status.success() {
            return Err(cargo_metadata_failure(
                &manifest,
                output.status,
                &output.stderr,
            ));
        }

        let metadata = parse_metadata(&output.stdout)?;
        Self::discover_from_metadata(root, metadata)
    }

    #[cfg(test)]
    pub(super) fn for_test(root: &Path, specifications: Vec<Specification>) -> Self {
        Self {
            root: root.to_path_buf(),
            context_path: root.join(".agents/context"),
            specifications,
        }
    }

    fn discover_from_metadata(root: PathBuf, metadata: CargoMetadata) -> Result<Self> {
        let workspace_members: HashSet<_> = metadata.workspace_members.into_iter().collect();
        let resolved_packages: HashSet<_> = metadata
            .resolve
            .map(|resolve| {
                resolve
                    .nodes
                    .into_iter()
                    .map(|node| node.package_id)
                    .collect()
            })
            .unwrap_or_default();

        let mut candidates = Vec::new();
        for package in metadata.packages {
            if workspace_members.contains(&package.package_id)
                || (!resolved_packages.is_empty()
                    && !resolved_packages.contains(&package.package_id))
            {
                continue;
            }

            let Some(package_root) = package.manifest_path.parent() else {
                continue;
            };
            let context_path = package_root.join("context");
            match fs::symlink_metadata(&context_path) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(Error::new(format!(
                        "cannot inspect {}: {error}",
                        context_path.display()
                    )));
                }
            };
            candidates.push(Specification {
                name: package.name,
                version: package.version,
                description: package.description,
                context_path,
                selector: String::new(),
            });
        }

        let mut name_counts = HashMap::new();
        for specification in &candidates {
            *name_counts
                .entry(specification.name.clone())
                .or_insert(0usize) += 1;
        }
        for specification in &mut candidates {
            specification.selector = if name_counts[&specification.name] == 1 {
                specification.name.clone()
            } else {
                format!("{}@{}", specification.name, specification.version)
            };
        }
        candidates.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.version.cmp(&right.version))
        });

        Ok(Self {
            context_path: root.join(".agents/context"),
            root,
            specifications: candidates,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn context_path(&self) -> &Path {
        &self.context_path
    }

    pub fn specifications(&self) -> &[Specification] {
        &self.specifications
    }

    pub fn find_specification(&self, selector: &str) -> Result<Option<Specification>> {
        let matches: Vec<_> = self
            .specifications
            .iter()
            .filter(|specification| {
                specification.selector == selector || specification.name == selector
            })
            .collect();

        match matches.as_slice() {
            [] => Ok(None),
            [specification] => Ok(Some((*specification).clone())),
            _ => {
                let selectors = matches
                    .iter()
                    .map(|specification| specification.selector.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(Error::new(format!(
                    "multiple versions of crate {selector:?} provide context; choose one of: {selectors}"
                )))
            }
        }
    }

    /// Load providers with context directories, including empty providers.
    pub fn providers(&self) -> Result<Vec<Package>> {
        self.specifications
            .iter()
            .cloned()
            .map(Package::load)
            .collect()
    }

    /// Load one provider by its package selector.
    pub fn find_provider(&self, selector: &str) -> Result<Option<Package>> {
        self.find_specification(selector)?
            .map(Package::load)
            .transpose()
    }

    /// Read the authored content of a selected definition.
    pub fn show_context_file(&self, selector: &str, file: &str) -> Result<Option<String>> {
        let Some(package) = self.find_provider(selector)? else {
            return Ok(None);
        };
        package
            .find_definition(file)?
            .map(Definition::content)
            .transpose()
    }

    /// Install context and skills from the same loaded provider documents.
    pub fn install(&self, selector: Option<&str>) -> Result<Installation> {
        // Resolve the requested scope and load each provider once:
        let loaded =
            if let Some(selector) = selector {
                vec![self.find_provider(selector)?.ok_or_else(|| {
                    Error::new(format!("no context found for crate {selector:?}"))
                })?]
            } else {
                self.providers()?
            };
        let definitions: Vec<_> = loaded
            .iter()
            .flat_map(|package| package.definitions.iter())
            .filter_map(|definition| match definition {
                Definition::Skill(skill) => Some(skill.clone()),
                Definition::File(_) => None,
            })
            .collect();
        let package = selector
            .and_then(|_| loaded.first())
            .map(|package| package.specification.name.as_str());

        // Apply skill installation rules, then write ordinary definitions:
        let skills = super::skill::install_definitions(&self.root, &definitions, package, true)?;
        let mut context_names = Vec::new();
        for package in loaded {
            if package.install_files(&self.context_path.join(package.specification.selector()))? {
                context_names.push(package.specification.selector().to_owned());
            }
        }
        Ok(Installation {
            context: context_names,
            skills,
        })
    }
}

fn run_cargo_metadata(root: &Path, cargo: &OsStr) -> Result<std::process::Output> {
    let manifest = root.join("Cargo.toml");
    let mut command = Command::new(cargo);
    command
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--manifest-path",
        ])
        .arg(&manifest)
        .current_dir(root);

    command.output().map_err(|error| {
        Error::new(format!(
            "cannot run cargo metadata for {}: {error}",
            manifest.display()
        ))
    })
}

fn cargo_executable(cargo: Option<OsString>) -> OsString {
    cargo.unwrap_or_else(|| "cargo".into())
}

fn cargo_metadata_failure(manifest: &Path, status: impl std::fmt::Display, stderr: &[u8]) -> Error {
    let details = String::from_utf8_lossy(stderr).trim().to_owned();
    Error::new(format!(
        "cargo metadata failed for {} ({}): {}",
        manifest.display(),
        status,
        if details.is_empty() {
            "run cargo check to resolve and lock the project's dependencies".to_owned()
        } else {
            details
        }
    ))
}

fn parse_metadata(stdout: &[u8]) -> Result<CargoMetadata> {
    serde_json::from_slice(stdout)
        .map_err(|error| Error::new(format!("cannot parse cargo metadata: {error}")))
}

#[derive(Deserialize)]
struct CargoMetadata {
    workspace_members: Vec<String>,
    packages: Vec<CargoPackage>,
    resolve: Option<Resolve>,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<ResolveNode>,
}

#[derive(Deserialize)]
struct ResolveNode {
    #[serde(rename = "id")]
    package_id: String,
}

#[derive(Deserialize)]
struct CargoPackage {
    #[serde(rename = "id")]
    package_id: String,
    name: String,
    version: String,
    description: Option<String>,
    manifest_path: PathBuf,
}

fn read_context_file(path: &Path) -> Result<String> {
    fs::read_to_string(path)
        .map_err(|error| Error::new(format!("cannot read {}: {error}", path.display())))
}

pub(crate) fn remove_existing(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::new(format!(
                "cannot inspect {}: {error}",
                path.display()
            )));
        }
    };
    let result = if metadata.file_type().is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| Error::new(format!("cannot remove {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cargo_package(id: &str, name: &str, version: &str, manifest_path: PathBuf) -> CargoPackage {
        CargoPackage {
            package_id: id.to_owned(),
            name: name.to_owned(),
            version: version.to_owned(),
            description: Some(format!("{name} description")),
            manifest_path,
        }
    }

    #[test]
    fn reports_cargo_execution_status_and_json_errors() {
        let directory = tempdir().unwrap();
        let error = Installer::discover(directory.path()).err().unwrap();
        assert!(error.to_string().contains("cargo metadata failed"));

        let error = Installer::discover_with_cargo(
            directory.path().to_path_buf(),
            directory.path().join("missing-cargo").into_os_string(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("cannot run cargo metadata"));

        assert_eq!(cargo_executable(None), OsString::from("cargo"));
        assert_eq!(
            cargo_executable(Some(OsString::from("custom-cargo"))),
            OsString::from("custom-cargo")
        );

        let error = cargo_metadata_failure(
            &directory.path().join("Cargo.toml"),
            "exit status: 1",
            b"  cargo stderr \n",
        );
        assert!(error.to_string().ends_with("cargo stderr"));
        let error = cargo_metadata_failure(&PathBuf::from("Cargo.toml"), "exit status: 1", b"\n");
        assert!(error.to_string().contains("run cargo check to resolve"));

        let error = parse_metadata(b"not JSON").err().unwrap();
        assert!(error.to_string().contains("cannot parse cargo metadata"));
    }

    #[cfg(unix)]
    #[test]
    fn reports_metadata_parse_errors_from_cargo() {
        let directory = tempdir().unwrap();
        let error =
            Installer::discover_with_cargo(directory.path().to_path_buf(), "/usr/bin/true".into())
                .unwrap_err();
        assert!(error.to_string().contains("cannot parse cargo metadata"));
    }

    #[test]
    fn discovers_only_resolved_non_workspace_context_packages_and_disambiguates_versions() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        for path in [
            "workspace",
            "duplicate-v1",
            "duplicate-v2",
            "missing",
            "file",
        ] {
            fs::create_dir_all(root.join(path)).unwrap();
        }
        fs::create_dir(root.join("workspace/context")).unwrap();
        fs::create_dir(root.join("duplicate-v1/context")).unwrap();
        fs::create_dir(root.join("duplicate-v2/context")).unwrap();
        fs::write(root.join("file/context"), "not a directory").unwrap();

        let packages = vec![
            cargo_package(
                "root 0.1.0",
                "workspace",
                "0.1.0",
                root.join("workspace/Cargo.toml"),
            ),
            cargo_package(
                "dup 1.0.0",
                "duplicate",
                "1.0.0",
                root.join("duplicate-v1/Cargo.toml"),
            ),
            cargo_package(
                "dup 2.0.0",
                "duplicate",
                "2.0.0",
                root.join("duplicate-v2/Cargo.toml"),
            ),
            cargo_package(
                "unresolved 1.0.0",
                "unresolved",
                "1.0.0",
                root.join("missing/Cargo.toml"),
            ),
            cargo_package("file 1.0.0", "file", "1.0.0", root.join("file/Cargo.toml")),
        ];
        let metadata = CargoMetadata {
            workspace_members: vec!["root 0.1.0".to_owned()],
            packages,
            resolve: Some(Resolve {
                nodes: vec![
                    ResolveNode {
                        package_id: "root 0.1.0".to_owned(),
                    },
                    ResolveNode {
                        package_id: "dup 1.0.0".to_owned(),
                    },
                    ResolveNode {
                        package_id: "dup 2.0.0".to_owned(),
                    },
                    ResolveNode {
                        package_id: "file 1.0.0".to_owned(),
                    },
                ],
            }),
        };
        let installer = Installer::discover_from_metadata(root.to_path_buf(), metadata).unwrap();

        assert_eq!(installer.root(), root);
        assert_eq!(installer.context_path(), root.join(".agents/context"));
        assert_eq!(
            installer
                .specifications()
                .iter()
                .map(Specification::selector)
                .collect::<Vec<_>>(),
            ["duplicate@1.0.0", "duplicate@2.0.0"]
        );
        assert!(installer.find_specification("missing").unwrap().is_none());
        assert!(
            installer
                .find_specification("duplicate")
                .unwrap_err()
                .to_string()
                .contains("duplicate@1.0.0, duplicate@2.0.0")
        );
        assert_eq!(
            installer
                .find_specification("duplicate@2.0.0")
                .unwrap()
                .unwrap()
                .version,
            "2.0.0"
        );
    }

    #[test]
    fn supports_metadata_without_a_resolve_graph_and_skips_missing_or_malformed_paths() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        fs::create_dir_all(root.join("valid/context")).unwrap();
        fs::create_dir(root.join("path-is-file")).unwrap();
        fs::write(root.join("path-is-file/context"), "not a directory").unwrap();

        let metadata = CargoMetadata {
            workspace_members: Vec::new(),
            packages: vec![
                cargo_package("valid", "valid", "1.0.0", root.join("valid/Cargo.toml")),
                cargo_package(
                    "missing",
                    "missing",
                    "1.0.0",
                    root.join("absent/Cargo.toml"),
                ),
                cargo_package(
                    "file",
                    "file",
                    "1.0.0",
                    root.join("path-is-file/Cargo.toml"),
                ),
                cargo_package("empty", "empty", "1.0.0", PathBuf::new()),
            ],
            resolve: None,
        };
        let installer = Installer::discover_from_metadata(root.to_path_buf(), metadata).unwrap();
        assert_eq!(installer.specifications().len(), 1);
        assert_eq!(installer.specifications()[0].selector(), "valid");
    }

    #[cfg(unix)]
    #[test]
    fn reports_context_provider_path_inspection_errors() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let blocker = root.join("blocker");
        fs::write(&blocker, "file").unwrap();
        let metadata = CargoMetadata {
            workspace_members: Vec::new(),
            packages: vec![cargo_package(
                "broken",
                "broken",
                "1.0.0",
                blocker.join("Cargo.toml"),
            )],
            resolve: None,
        };
        assert!(Installer::discover_from_metadata(root.to_path_buf(), metadata).is_err());
    }

    #[test]
    fn finds_loaded_source_paths_before_skill_names() {
        let directory = tempdir().unwrap();
        let context = directory.path().join("context");
        fs::create_dir_all(context.join("nested")).unwrap();
        fs::write(context.join("guide.md"), "guide").unwrap();
        let source =
            "---\nname: guide\nmetadata: {type: skill}\ndescription: Workflow.\n---\n# Workflow\n";
        fs::write(context.join("workflow.md"), source).unwrap();
        fs::write(context.join("nested/readme.MD"), "nested").unwrap();
        let specification = Specification::for_test("provider", "1.0.0", context.clone());
        let installer = Installer::for_test(directory.path(), vec![specification]);
        let package = installer.find_provider("provider").unwrap().unwrap();
        assert_eq!(
            package
                .find_definition("guide")
                .unwrap()
                .unwrap()
                .content()
                .unwrap(),
            "guide"
        );
        assert_eq!(
            package
                .find_definition("guide.md")
                .unwrap()
                .unwrap()
                .content()
                .unwrap(),
            "guide"
        );
        assert_eq!(
            package
                .find_definition("workflow")
                .unwrap()
                .unwrap()
                .content()
                .unwrap(),
            source
        );
        assert_eq!(
            package
                .find_definition("nested/readme.MD")
                .unwrap()
                .unwrap()
                .content()
                .unwrap(),
            "nested"
        );
        assert!(package.find_definition("not-found").unwrap().is_none());
        for path in ["", "/etc/passwd", "./guide.md", "nested/../guide.md"] {
            assert!(package.find_definition(path).is_err());
        }

        // Source lookup uses the loaded snapshot, with installed names as a fallback:
        fs::remove_file(context.join("guide.md")).unwrap();
        assert_eq!(
            package
                .find_definition("guide")
                .unwrap()
                .unwrap()
                .content()
                .unwrap(),
            "guide"
        );
        assert_eq!(
            installer
                .show_context_file("provider", "guide")
                .unwrap()
                .as_deref(),
            Some(source)
        );
        fs::write(context.join("guide"), "exact source").unwrap();
        fs::write(context.join("guide.md"), "markdown source").unwrap();
        assert_eq!(
            installer
                .show_context_file("provider", "guide")
                .unwrap()
                .as_deref(),
            Some("exact source")
        );
        assert!(installer.find_provider("missing").unwrap().is_none());
        assert!(read_context_file(&context).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn reports_context_removal_errors() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().unwrap();
        let parent = directory.path().join("context");
        fs::create_dir(&parent).unwrap();
        let destination = parent.join("installed");
        fs::write(&destination, "previous context").unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o555)).unwrap();

        let result = remove_existing(&destination);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.unwrap_err().to_string().contains("cannot remove"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_context_file_symlinks_that_escape_the_provider() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let context = directory.path().join("context");
        fs::create_dir_all(&context).unwrap();
        let outside = directory.path().join("private.md");
        fs::write(&outside, "secret").unwrap();
        symlink(&outside, context.join("external.md")).unwrap();

        let package = Package::load(Specification::for_test("provider", "1.0.0", context)).unwrap();
        assert!(package.find_definition("external.md").unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn reports_removal_errors_for_paths_beneath_files() {
        let directory = tempdir().unwrap();
        let blocker = directory.path().join("blocker");
        fs::write(&blocker, "file").unwrap();
        assert!(remove_existing(&blocker.join("child")).is_err());
    }
    #[test]
    fn coordinates_ordinary_files_skills_and_empty_providers() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let context = root.join("provider/context");
        fs::create_dir_all(&context).unwrap();
        fs::write(context.join("guide.md"), "# Guide\n").unwrap();
        fs::write(context.join("data.txt"), "Opaque resource").unwrap();
        fs::write(
            context.join("workflow.md"),
            "---\nmetadata: {type: skill}\ndescription: Workflow.\n---\n",
        )
        .unwrap();
        let package = Specification::for_test("provider", "1.0.0", context.clone());
        let installer = Installer::for_test(root, vec![package.clone()]);
        assert_eq!(
            installer
                .show_context_file("provider", "data.txt")
                .unwrap()
                .as_deref(),
            Some("Opaque resource")
        );
        assert!(
            installer
                .show_context_file("provider", "provider-workflow")
                .unwrap()
                .unwrap()
                .contains("Workflow.")
        );
        assert!(
            installer
                .install(Some("missing"))
                .unwrap_err()
                .to_string()
                .contains("no context found")
        );
        let ambiguous = Installer::for_test(root, vec![package.clone(), package]);
        assert!(ambiguous.install(Some("provider")).is_err());
        let loaded = installer.install(None).unwrap();
        assert_eq!(loaded.context, ["provider@1.0.0"]);
        assert_eq!(loaded.skills, ["provider-workflow"]);
        fs::remove_file(context.join("guide.md")).unwrap();
        fs::remove_file(context.join("data.txt")).unwrap();
        assert!(installer.install(None).unwrap().context.is_empty());
        assert!(installer.install(None).unwrap().context.is_empty());
        fs::write(context.join("broken.md"), "---\n[invalid\n---\n").unwrap();
        assert!(
            installer
                .show_context_file("provider", "provider-workflow")
                .is_err()
        );
        assert!(installer.install(Some("provider")).is_err());
    }

    #[test]
    fn reports_ordinary_context_write_failures_from_the_main_installation() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let context = root.join("provider/context");
        fs::create_dir_all(&context).unwrap();
        fs::write(context.join("guide.md"), "# Guide\n").unwrap();
        let package = Specification::for_test("provider", "1.0.0", context);
        let installer = Installer::for_test(root, vec![package]);
        let destination = root.join(".agents/context/provider@1.0.0/guide.md");
        let _failure = fs::fail_once(fs::Operation::Write, move |path| path == destination);
        assert!(
            installer
                .install(None)
                .unwrap_err()
                .to_string()
                .contains("cannot write")
        );
    }
}
