// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::package_context::PackageContext;
#[cfg(test)]
use super::test_filesystem as fs;
use bake::{Error, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{OsStr, OsString};
#[cfg(not(test))]
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

/// A resolved Cargo package that provides a top-level `context/` directory.
#[derive(Clone, Debug)]
pub struct ContextPackage {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub context_path: PathBuf,
    selector: String,
}

impl ContextPackage {
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

/// A context file relative to its provider's `context/` directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextFile {
    pub path: PathBuf,
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
    packages: Vec<ContextPackage>,
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
    pub(super) fn for_test(root: &Path, packages: Vec<ContextPackage>) -> Self {
        Self {
            root: root.to_path_buf(),
            context_path: root.join(".agents/context"),
            packages,
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
            candidates.push(ContextPackage {
                name: package.name,
                version: package.version,
                description: package.description,
                context_path,
                selector: String::new(),
            });
        }

        let mut name_counts = HashMap::new();
        for package in &candidates {
            *name_counts.entry(package.name.clone()).or_insert(0usize) += 1;
        }
        for package in &mut candidates {
            package.selector = if name_counts[&package.name] == 1 {
                package.name.clone()
            } else {
                format!("{}@{}", package.name, package.version)
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
            packages: candidates,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn context_path(&self) -> &Path {
        &self.context_path
    }

    pub fn packages(&self) -> &[ContextPackage] {
        &self.packages
    }

    pub fn find_package(&self, selector: &str) -> Result<Option<ContextPackage>> {
        let matches: Vec<_> = self
            .packages
            .iter()
            .filter(|package| package.selector == selector || package.name == selector)
            .collect();

        match matches.as_slice() {
            [] => Ok(None),
            [package] => Ok(Some((*package).clone())),
            _ => {
                let selectors = matches
                    .iter()
                    .map(|package| package.selector.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(Error::new(format!(
                    "multiple versions of crate {selector:?} provide context; choose one of: {selectors}"
                )))
            }
        }
    }

    /// List ordinary context from one loaded provider.
    pub fn list_context_files(&self, package: &ContextPackage) -> Result<Vec<ContextFile>> {
        let context = PackageContext::load(package)?;
        Ok(context
            .files
            .keys()
            .map(|path| ContextFile {
                path: path
                    .strip_prefix(&package.context_path)
                    .expect("context files belong to their package")
                    .to_owned(),
            })
            .collect())
    }

    /// Show an ordinary document or opaque resource from a provider.
    pub fn show_context_file(&self, selector: &str, file: &str) -> Result<Option<String>> {
        let Some(package) = self.find_package(selector)? else {
            return Ok(None);
        };
        let Some((root, path)) = find_context_file_with_root(&package.context_path, file)? else {
            return Ok(None);
        };
        let context = PackageContext::load(&package)?;
        let relative = path
            .strip_prefix(root)
            .expect("resolved context paths stay within their root");
        match context.files.get(&package.context_path.join(relative)) {
            Some(Some(document)) => Ok(Some(document.content.clone())),
            Some(None) => read_context_file(&path),
            None => Ok(None),
        }
    }

    /// Refresh ordinary context for one provider.
    pub fn install_package(&self, selector: &str) -> Result<bool> {
        let Some(package) = self.find_package(selector)? else {
            return Ok(false);
        };
        PackageContext::load(&package)?
            .install(&package, &self.context_path.join(package.selector()))
    }

    /// Refresh ordinary context for every resolved provider.
    pub fn install_all(&self) -> Result<Vec<String>> {
        let mut installed = Vec::new();
        for package in &self.packages {
            if self.install_package(package.selector())? {
                installed.push(package.selector().to_owned());
            }
        }
        Ok(installed)
    }

    /// Install context and skills from the same loaded provider documents.
    pub fn install(&self, selector: Option<&str>) -> Result<Installation> {
        // Resolve the requested scope and load each provider once:
        let packages =
            if let Some(selector) = selector {
                vec![self.find_package(selector)?.ok_or_else(|| {
                    Error::new(format!("no context found for crate {selector:?}"))
                })?]
            } else {
                self.packages.clone()
            };
        let loaded = packages
            .iter()
            .map(|package| PackageContext::load(package).map(|context| (package, context)))
            .collect::<Result<Vec<_>>>()?;
        let definitions: Vec<_> = loaded
            .iter()
            .flat_map(|(_, context)| context.skills.clone())
            .collect();
        let package = selector
            .and_then(|_| packages.first())
            .map(|package| package.name.as_str());

        // Refresh skills and ordinary files from those loaded representations:
        let skills = super::skill::install_definitions(&self.root, &definitions, package, true)?;
        let mut context = Vec::new();
        for (package, files) in loaded {
            if files.install(package, &self.context_path.join(package.selector()))? {
                context.push(package.selector().to_owned());
            }
        }
        Ok(Installation { context, skills })
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

#[cfg(test)]
fn find_context_file(context_path: &Path, file: &str) -> Result<Option<PathBuf>> {
    Ok(find_context_file_with_root(context_path, file)?.map(|(_, path)| path))
}

fn find_context_file_with_root(
    context_path: &Path,
    file: &str,
) -> Result<Option<(PathBuf, PathBuf)>> {
    let requested = Path::new(file);
    if requested.is_absolute()
        || requested
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::new(
            "context file must be a relative path inside context/",
        ));
    }

    let canonical_root = canonical_context_root(context_path)?;
    let mut candidates = vec![context_path.join(requested)];
    if requested.extension().is_none() {
        candidates.push(context_path.join(requested).with_extension("md"));
    }

    for candidate in candidates {
        let Ok(canonical_candidate) = candidate.canonicalize() else {
            continue;
        };
        if !canonical_candidate.starts_with(&canonical_root) || !canonical_candidate.is_file() {
            continue;
        }
        return Ok(Some((canonical_root, canonical_candidate)));
    }

    Ok(None)
}

fn canonical_context_root(context_path: &Path) -> Result<PathBuf> {
    context_path.canonicalize().map_err(|error| {
        Error::new(format!(
            "cannot resolve {}: {error}",
            context_path.display()
        ))
    })
}

fn read_context_file(path: &Path) -> Result<Option<String>> {
    fs::read_to_string(path)
        .map(Some)
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
                .packages()
                .iter()
                .map(ContextPackage::selector)
                .collect::<Vec<_>>(),
            ["duplicate@1.0.0", "duplicate@2.0.0"]
        );
        assert!(installer.find_package("missing").unwrap().is_none());
        assert!(
            installer
                .find_package("duplicate")
                .unwrap_err()
                .to_string()
                .contains("duplicate@1.0.0, duplicate@2.0.0")
        );
        assert_eq!(
            installer
                .find_package("duplicate@2.0.0")
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
        assert_eq!(installer.packages().len(), 1);
        assert_eq!(installer.packages()[0].selector(), "valid");
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
    fn locates_context_files_safely_and_finds_markdown_extensions() {
        let directory = tempdir().unwrap();
        let context = directory.path().join("context");
        fs::create_dir_all(context.join("nested")).unwrap();
        fs::write(context.join("guide.md"), "guide").unwrap();
        fs::write(context.join("nested/readme.MD"), "nested").unwrap();
        fs::create_dir(context.join("directory")).unwrap();
        fs::write(context.join("plain.txt"), "plain").unwrap();

        assert_eq!(
            find_context_file(&context, "guide").unwrap(),
            Some(context.join("guide.md").canonicalize().unwrap())
        );
        assert!(
            find_context_file(&context, "nested/readme.MD")
                .unwrap()
                .is_some()
        );
        assert_eq!(find_context_file(&context, "not-found").unwrap(), None);
        assert_eq!(find_context_file(&context, "directory").unwrap(), None);
        assert!(find_context_file(&context, "/etc/passwd").is_err());
        assert!(find_context_file(&context, "./guide.md").is_err());
        assert!(find_context_file(&context, "nested/../guide.md").is_err());
        assert!(find_context_file(&context.join("missing"), "guide.md").is_err());
        assert!(canonical_context_root(&context.join("does-not-exist")).is_err());
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

        assert_eq!(find_context_file(&context, "external.md").unwrap(), None);
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
        let package = ContextPackage::for_test("provider", "1.0.0", context.clone());
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
                .show_context_file("provider", "workflow")
                .unwrap()
                .is_none()
        );
        assert!(!installer.install_package("missing").unwrap());
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
        assert_eq!(loaded.skills, ["provider-workflow (provider@1.0.0)"]);
        fs::remove_file(context.join("guide.md")).unwrap();
        fs::remove_file(context.join("data.txt")).unwrap();
        assert!(installer.install_all().unwrap().is_empty());
        assert!(installer.install(None).unwrap().context.is_empty());
        fs::write(context.join("broken.md"), "---\n[invalid\n---\n").unwrap();
        assert!(installer.show_context_file("provider", "workflow").is_err());
        assert!(installer.install_package("provider").is_err());
        assert!(installer.install_all().is_err());
    }

    #[test]
    fn reports_ordinary_context_write_failures_from_the_main_installation() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let context = root.join("provider/context");
        fs::create_dir_all(&context).unwrap();
        fs::write(context.join("guide.md"), "# Guide\n").unwrap();
        let package = ContextPackage::for_test("provider", "1.0.0", context);
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
