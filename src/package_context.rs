// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::document::ContextDocument;
use super::installer::ContextPackage;
use super::skill::Skill;
#[cfg(test)]
use super::test_filesystem as fs;
use bake::{Error, Result};
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::fs;
use std::path::{Path, PathBuf};

/// Loaded ordinary context and skills from one resolved provider.
pub(crate) struct PackageContext {
    pub(crate) files: BTreeMap<PathBuf, Option<ContextDocument>>,
    pub(crate) skills: Vec<Skill>,
}

impl PackageContext {
    /// Classify root documents before descending into companion resource directories.
    pub(crate) fn load(package: &ContextPackage) -> Result<Self> {
        let mut context = Self {
            files: BTreeMap::new(),
            skills: Vec::new(),
        };
        context.load_directory(package, &package.context_path)?;
        Ok(context)
    }

    fn load_directory(&mut self, package: &ContextPackage, directory: &Path) -> Result<()> {
        // Load Markdown declarations before interpreting subdirectories:
        let entries = fs::read_dir(directory)
            .map_err(|error| Error::new(format!("cannot read {}: {error}", directory.display())))?;
        let mut directories = Vec::new();
        for entry in entries {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                directories.push(path);
            } else if kind.is_file() {
                let document = if path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
                {
                    Some(ContextDocument::load(&path)?)
                } else {
                    None
                };
                if let Some(document) = document.as_ref()
                    && document.is_skill()?
                {
                    self.skills.push(Skill::from_document(package, document)?);
                } else {
                    self.files.insert(path, document);
                }
            }
        }

        // Companion resources belong to the skill and remain opaque:
        for directory in directories {
            if !self
                .skills
                .iter()
                .any(|skill| directory == package.context_path.join(&skill.source_name))
            {
                self.load_directory(package, &directory)?;
            }
        }
        self.skills
            .sort_by(|left, right| left.name.cmp(&right.name));
        Ok(())
    }

    /// Refresh ordinary context from loaded Markdown and opaque resources.
    pub(crate) fn install(&self, package: &ContextPackage, destination: &Path) -> Result<bool> {
        super::installer::remove_existing(destination)?;
        for (source, document) in &self.files {
            let relative = source
                .strip_prefix(&package.context_path)
                .expect("context files belong to their package");
            let target = destination.join(relative);
            fs::create_dir_all(target.parent().unwrap()).map_err(|error| {
                Error::new(format!("cannot create {}: {error}", target.display()))
            })?;
            if let Some(document) = document {
                document.write(&target)?;
            } else {
                fs::copy(source, &target).map_err(|error| {
                    Error::new(format!(
                        "cannot copy {} to {}: {error}",
                        source.display(),
                        target.display()
                    ))
                })?;
            }
        }
        Ok(!self.files.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_filesystem::{Operation, fail_once};
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn installs_loaded_documents_and_opaque_assets() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("context");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("guide.md"), "# Original\n").unwrap();
        fs::write(source.join("image.bin"), [0, 255]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("guide.md", source.join("linked.md")).unwrap();
        let package = ContextPackage::for_test("provider", "1.0.0", source.clone());
        let loaded = PackageContext::load(&package).unwrap();
        fs::write(source.join("guide.md"), "# Changed\n").unwrap();
        let destination = directory.path().join("installed");
        loaded.install(&package, &destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("guide.md")).unwrap(),
            "# Original\n"
        );
        assert_eq!(fs::read(destination.join("image.bin")).unwrap(), [0, 255]);
        assert!(!destination.join("linked.md").exists());
        for (operation, path) in [
            (Operation::CreateDirectoryTree, destination.clone()),
            (Operation::Write, destination.join("guide.md")),
            (Operation::Copy, source.join("image.bin")),
        ] {
            let _failure = fail_once(operation, move |candidate| candidate == path);
            assert!(
                loaded
                    .install(&package, &destination)
                    .unwrap_err()
                    .to_string()
                    .contains("injected filesystem failure")
            );
        }
    }

    #[test]
    fn reports_provider_traversal_errors() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("context");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("guide.md"), "# Guide\n").unwrap();
        let package = ContextPackage::for_test("provider", "1.0.0", source.clone());
        for (operation, path) in [
            (Operation::ReadDirectory, source.clone()),
            (Operation::ReadDirectoryEntry, source.clone()),
            (Operation::FileType, source.join("guide.md")),
        ] {
            let _failure = fail_once(operation, move |candidate| candidate == path);
            assert!(
                PackageContext::load(&package)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("injected filesystem failure")
            );
        }
    }
}
