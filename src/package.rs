// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::document::Document;
use super::installer::{Definition, FileDefinition, Specification};
use super::skill::SkillDefinition;
#[cfg(test)]
use super::test_filesystem as fs;
use bake::{Error, Result};
#[cfg(not(test))]
use std::fs;
use std::path::{Component, Path};

/// A loaded provider and its context definitions.
pub struct Package {
    pub specification: Specification,
    pub definitions: Vec<Definition>,
}

impl Package {
    /// Classify root documents before descending into companion resource directories.
    pub fn load(specification: Specification) -> Result<Self> {
        let mut package = Self {
            specification,
            definitions: Vec::new(),
        };
        let context_path = package.specification.context_path.clone();
        Self::load_directory(
            &package.specification,
            &mut package.definitions,
            &context_path,
        )?;
        package
            .definitions
            .sort_by(|left, right| left.path().cmp(right.path()));
        Ok(package)
    }

    fn load_directory(
        specification: &Specification,
        definitions: &mut Vec<Definition>,
        directory: &Path,
    ) -> Result<()> {
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
                    Some(Document::load(&path)?)
                } else {
                    None
                };
                if let Some(document) = document.as_ref()
                    && document.is_skill()
                {
                    definitions.push(Definition::Skill(SkillDefinition::from_document(
                        specification,
                        document,
                    )?));
                } else {
                    let relative = path
                        .strip_prefix(&specification.context_path)
                        .expect("context files belong to their package")
                        .to_owned();
                    definitions.push(Definition::File(FileDefinition {
                        path: relative,
                        source_path: path,
                        document,
                    }));
                }
            }
        }

        // Companion resources belong to the skill and remain opaque:
        for directory in directories {
            if !definitions.iter().any(|definition| match definition {
                Definition::Skill(skill) => {
                    directory
                        == specification
                            .context_path
                            .join(skill.path.file_stem().unwrap())
                }
                Definition::File(_) => false,
            }) {
                Self::load_directory(specification, definitions, &directory)?;
            }
        }
        Ok(())
    }

    /// Find a source path, optionally omitting .md, then an installed skill name.
    pub fn find_definition(&self, file: &str) -> Result<Option<&Definition>> {
        let requested = Path::new(file);
        if file.is_empty()
            || requested.is_absolute()
            || requested
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(Error::new(
                "context file must be a relative path inside context/",
            ));
        }

        let mut candidates = vec![requested.to_owned()];
        if requested.extension().is_none() {
            candidates.push(requested.with_extension("md"));
        }
        for path in candidates {
            if let Some(definition) = self.definitions.iter().find(|item| item.path() == path) {
                return Ok(Some(definition));
            }
        }
        Ok(self
            .definitions
            .iter()
            .find(|item| matches!(item, Definition::Skill(skill) if skill.name == file)))
    }

    /// Refresh ordinary definitions from loaded Markdown and opaque resources.
    pub(crate) fn install_files(&self, destination: &Path) -> Result<bool> {
        super::installer::remove_existing(destination)?;
        let files: Vec<_> = self
            .definitions
            .iter()
            .filter_map(|definition| match definition {
                Definition::File(file) => Some(file),
                Definition::Skill(_) => None,
            })
            .collect();

        for file in &files {
            let target = destination.join(&file.path);
            fs::create_dir_all(target.parent().unwrap()).map_err(|error| {
                Error::new(format!("cannot create {}: {error}", target.display()))
            })?;
            if let Some(document) = &file.document {
                document.write(&target)?;
            } else {
                fs::copy(&file.source_path, &target).map_err(|error| {
                    Error::new(format!(
                        "cannot copy {} to {}: {error}",
                        file.source_path.display(),
                        target.display()
                    ))
                })?;
            }
        }
        Ok(!files.is_empty())
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
        let specification = Specification::for_test("provider", "1.0.0", source.clone());
        let loaded = Package::load(specification).unwrap();
        assert_eq!(loaded.specification.name, "provider");
        fs::write(source.join("guide.md"), "# Changed\n").unwrap();
        let destination = directory.path().join("installed");
        loaded.install_files(&destination).unwrap();
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
                    .install_files(&destination)
                    .unwrap_err()
                    .to_string()
                    .contains("injected filesystem failure")
            );
        }
    }

    #[test]
    fn preserves_skill_source_paths_and_empty_providers() {
        let directory = tempdir().unwrap();
        let context = directory.path().join("context");
        fs::create_dir(&context).unwrap();
        let specification = Specification::for_test("provider", "1.0.0", context.clone());
        assert!(
            Package::load(specification.clone())
                .unwrap()
                .definitions
                .is_empty()
        );
        let source = "---\nmetadata: {type: skill}\nname: installed-name\ndescription: Workflow.\n---\n# Workflow\n";
        fs::write(context.join("workflow.MD"), source).unwrap();
        let package = Package::load(specification).unwrap();
        let definition = package.find_definition("workflow.MD").unwrap().unwrap();
        assert_eq!(definition.path(), Path::new("workflow.MD"));
        assert_eq!(definition.content().unwrap(), source);
        assert_eq!(
            package
                .find_definition("installed-name")
                .unwrap()
                .unwrap()
                .content()
                .unwrap(),
            source
        );
    }

    #[test]
    fn reports_provider_traversal_errors() {
        let directory = tempdir().unwrap();
        let source = directory.path().join("context");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("guide.md"), "# Guide\n").unwrap();
        let specification = Specification::for_test("provider", "1.0.0", source.clone());
        for (operation, path) in [
            (Operation::ReadDirectory, source.clone()),
            (Operation::ReadDirectoryEntry, source.clone()),
            (Operation::FileType, source.join("guide.md")),
        ] {
            let _failure = fail_once(operation, move |candidate| candidate == path);
            assert!(
                Package::load(specification.clone())
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("injected filesystem failure")
            );
        }
    }
}
