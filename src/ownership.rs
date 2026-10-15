// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

use super::document::Document;
use super::skill::validate_skill_name;
#[cfg(test)]
use super::test_filesystem as filesystem;
use bake::{Error, Result};
use serde_yaml_ng::Value;
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::fs as filesystem;
use std::path::Path;

const OWNERSHIP_FILE: &str = "SKILL.md";

/// Provider identity recorded in installed skill front matter.
#[derive(Clone, Debug)]
pub(crate) struct SkillOwner {
    pub(crate) ecosystem: String,
    pub(crate) package: String,
}

fn load_skill_owner(directory: &Path) -> Result<Option<SkillOwner>> {
    let path = directory.join(OWNERSHIP_FILE);
    match filesystem::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err(Error::new(format!(
                "skill ownership {} is not a regular file",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(Error::new(format!(
                "cannot inspect {}: {error}",
                path.display()
            )));
        }
    }
    let document = Document::load(&path)?;
    let Some(metadata) = document.metadata() else {
        return Ok(None);
    };
    let fields = ["ecosystem", "package", "version"];
    if metadata.get("type").and_then(Value::as_str) != Some("skill")
        || !metadata.contains_key("ecosystem")
    {
        return Ok(None);
    }
    if !fields.iter().all(|field| {
        metadata
            .get(*field)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        return Err(Error::new(format!(
            "invalid skill ownership in {}",
            path.display()
        )));
    }
    let owner = SkillOwner {
        ecosystem: metadata["ecosystem"].as_str().unwrap().to_owned(),
        package: metadata["package"].as_str().unwrap().to_owned(),
    };
    Ok(Some(owner))
}

pub(crate) fn installed_skill_owners(root: &Path) -> Result<BTreeMap<String, SkillOwner>> {
    match filesystem::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            return Err(Error::new(format!(
                "skill installation path {} is not a regular directory",
                root.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(Error::new(format!(
                "cannot inspect {}: {error}",
                root.display()
            )));
        }
    }
    let entries = filesystem::read_dir(root)
        .map_err(|error| Error::new(format!("cannot read {}: {error}", root.display())))?;
    let mut owners = BTreeMap::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(owner) = load_skill_owner(&entry.path())? {
            let name = entry.file_name().to_string_lossy().into_owned();
            validate_skill_name(&name)?;
            owners.insert(name, owner);
        }
    }
    Ok(owners)
}

#[cfg(test)]
mod tests {
    use super::super::test_filesystem::{Operation, fail_once};
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn distinguishes_project_metadata_from_complete_provider_provenance() {
        let directory = tempdir().unwrap();
        let skill = directory.path().join("workflow");
        fs::create_dir(&skill).unwrap();
        assert!(installed_skill_owners(directory.path()).unwrap().is_empty());
        fs::write(directory.path().join("readme.md"), "Project skills").unwrap();
        for metadata in [
            "version: '1.0'",
            "type: skill, version: '1.0'",
            "ecosystem: gem, package: provider, version: '1.0'",
        ] {
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nmetadata: {{{metadata}}}\n---\n"),
            )
            .unwrap();
            assert!(installed_skill_owners(directory.path()).unwrap().is_empty());
        }
        for metadata in [
            "type: skill, ecosystem: cargo",
            "type: skill, ecosystem: cargo, package: provider, version: 1",
            "type: skill, ecosystem: cargo, package: ' ', version: '1.0'",
        ] {
            fs::write(
                skill.join("SKILL.md"),
                format!("---\nmetadata: {{{metadata}}}\n---\n"),
            )
            .unwrap();
            assert!(
                installed_skill_owners(directory.path())
                    .unwrap_err()
                    .to_string()
                    .contains("invalid skill ownership")
            );
        }
        fs::write(
            skill.join("SKILL.md"),
            include_str!("../tests/fixtures/SKILL.md"),
        )
        .unwrap();
        assert_eq!(
            installed_skill_owners(directory.path()).unwrap()["workflow"].package,
            "provider"
        );
        fs::rename(&skill, directory.path().join("Bad_Name")).unwrap();
        assert!(
            installed_skill_owners(directory.path())
                .unwrap_err()
                .to_string()
                .contains("invalid skill name")
        );
    }

    #[test]
    fn reports_unreadable_or_non_regular_ownership_without_claiming_it() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let skill = root.join("workflow");
        fs::create_dir(&skill).unwrap();
        fs::create_dir(skill.join("SKILL.md")).unwrap();
        assert!(
            installed_skill_owners(root)
                .unwrap_err()
                .to_string()
                .contains("not a regular file")
        );
        fs::remove_dir(skill.join("SKILL.md")).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            include_str!("../tests/fixtures/SKILL.md"),
        )
        .unwrap();
        for (operation, path) in [
            (Operation::Inspect, root.to_owned()),
            (Operation::Inspect, skill.join("SKILL.md")),
            (Operation::ReadDirectory, root.to_owned()),
            (Operation::ReadDirectoryEntry, root.to_owned()),
            (Operation::FileType, skill.clone()),
        ] {
            let _failure = fail_once(operation, move |candidate| candidate == path);
            assert!(
                installed_skill_owners(root)
                    .unwrap_err()
                    .to_string()
                    .contains("injected filesystem failure")
            );
        }
    }
}
