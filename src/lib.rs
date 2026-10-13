// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

//! Bake tasks and APIs for consuming context directories shipped by Cargo packages.
mod document;
mod exclude;
mod installer;
mod ownership;
mod package;
mod skill;
#[cfg(test)]
mod test_filesystem;

pub use installer::{Definition, FileDefinition, Installation, Installer, Specification};
pub use package::Package;
pub use skill::SkillDefinition;

use bake::{Context, Error, Result};

fn installer(context: &mut Context) -> Result<Installer> {
    if let Some(installer) = context.get::<Installer>() {
        return Ok(installer.clone());
    }

    let installer = Installer::discover(context.root())?;
    context.insert(installer.clone());
    Ok(installer)
}

/// List dependency crates that ship context, or list one crate's definitions.
#[bake::task(name = "agent:context:list")]
pub fn list(context: &mut Context, package: Option<String>) -> Result<String> {
    let installer = installer(context)?;

    if let Some(package) = package {
        let Some(provider) = installer.find_provider(&package)? else {
            return Ok(format!("No context found for crate '{package}'"));
        };

        let package = Package::load(provider.clone())?;
        if package.definitions.is_empty() {
            return Ok(format!(
                "No context definitions found for crate '{}'.",
                provider.selector()
            ));
        }
        let mut output = format!("Context files for crate '{}':", provider.selector());
        for definition in package.definitions {
            output.push_str(&format!("\n  {}", definition.display_name()));
        }
        return Ok(output);
    }

    let mut providers = Vec::new();
    for provider in installer.providers() {
        if !Package::load(provider.clone())?.definitions.is_empty() {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        return Ok("No dependency context files found".to_owned());
    }

    let mut output = String::from("Crates with context available:");
    for provider in providers {
        output.push_str(&format!(
            "\n  {} ({})",
            provider.selector(),
            provider.version
        ));
    }
    Ok(output)
}

/// Show a context file from one resolved dependency crate.
#[bake::task(name = "agent:context:show")]
pub fn show(
    context: &mut Context,
    #[bake(named)] package: String,
    #[bake(named)] file: String,
) -> Result<String> {
    let installer = installer(context)?;
    let Some(content) = installer.show_context_file(&package, &file)? else {
        return Err(Error::new(format!(
            "context file {file:?} was not found in crate {package:?}"
        )));
    };
    Ok(content)
}

/// Install context and skills from one crate or all dependencies.
#[bake::task(name = "agent:context:install")]
pub fn install(context: &mut Context, package: Option<String>) -> Result<String> {
    let installer = installer(context)?;
    let installed = installer.install(package.as_deref())?;
    let installed_context = installed.context;
    let installed_skills = installed.skills;

    let mut output = Vec::new();
    if !installed_context.is_empty() {
        output.push(format!(
            "Installed context from: {}",
            installed_context.join(", ")
        ));
    }
    if !installed_skills.is_empty() {
        output.push(format!("Installed skills: {}", installed_skills.join(", ")));
    }

    if output.is_empty() {
        Ok("No dependency context or skills were installed".to_owned())
    } else {
        Ok(output.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn make_context(root: &Path, providers: Vec<Specification>) -> Context {
        let mut context = bake::Registry::new().context(root);
        context.insert(Installer::for_test(root, providers));
        context
    }

    fn provider(root: &Path, name: &str, version: &str) -> Specification {
        Specification::for_test(name, version, root.join(name).join("context"))
    }

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn task_wrappers_return_outputs_and_propagate_errors() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let guide = provider(root, "provider", "1.0.0");
        write(&guide.context_path.join("guide.md"), "# Guide\n");
        write(
            &guide.context_path.join("broken.md"),
            "---\nmetadata: [\n---\n# Invalid YAML\n",
        );
        let mut context = make_context(root, vec![guide.clone()]);

        assert!(
            list(&mut context, Some("unknown".to_owned()))
                .unwrap()
                .contains("No context found")
        );
        assert!(list(&mut context, None).is_err());
        assert!(list(&mut context, Some("provider".to_owned())).is_err());
        assert!(install(&mut context, None).is_err());
        assert!(show(&mut context, "provider".to_owned(), "missing.md".to_owned()).is_err());
        assert!(install(&mut context, Some("provider".to_owned())).is_err());

        let mut duplicates = make_context(
            root,
            vec![
                provider(root, "duplicate", "1.0.0"),
                provider(root, "duplicate", "2.0.0"),
            ],
        );
        assert!(list(&mut duplicates, Some("duplicate".to_owned())).is_err());
        assert!(
            show(
                &mut duplicates,
                "duplicate".to_owned(),
                "guide.md".to_owned()
            )
            .is_err()
        );
        assert!(
            duplicates
                .get::<Installer>()
                .unwrap()
                .install(Some("duplicate"))
                .is_err()
        );

        let listing_package = provider(root, "listing", "1.0.0");
        write(
            &listing_package.context_path.join("guide.md"),
            "# Listing\n",
        );
        let mut listing_context = make_context(root, vec![listing_package]);
        assert!(
            list(&mut listing_context, None)
                .unwrap()
                .contains("listing@1.0.0 (1.0.0)")
        );

        let empty_package = provider(root, "empty", "1.0.0");
        fs::create_dir_all(&empty_package.context_path).unwrap();
        let mut empty_context = make_context(root, vec![empty_package]);
        assert_eq!(
            list(&mut empty_context, Some("empty".to_owned())).unwrap(),
            "No context definitions found for crate 'empty@1.0.0'."
        );
        assert!(
            list(&mut empty_context, None)
                .unwrap()
                .contains("No dependency context files")
        );
    }

    #[test]
    fn task_wrappers_propagate_install_failures() {
        let directory = tempdir().unwrap();
        let root = directory.path();
        let provider = provider(root, "provider", "1.0.0");
        write(&provider.context_path.join("guide.md"), "# Guide\n");

        let ownership = root.join(".agents/skills/provider-workflow/SKILL.md");
        write(
            &ownership,
            "---\nmetadata:\n  type: skill\n  ecosystem: cargo\n  package: provider\n  version: 1.0.0\n---\n",
        );
        let failure_path = ownership.clone();
        let _failure = test_filesystem::fail_once(test_filesystem::Operation::Read, move |path| {
            path == failure_path
        });
        let mut context = make_context(root, vec![provider.clone()]);
        assert!(install(&mut context, Some("provider".to_owned())).is_err());
        drop(_failure);

        let mut empty = make_context(root, Vec::new());
        assert!(
            list(&mut empty, None)
                .unwrap()
                .contains("No dependency context files")
        );
    }

    #[test]
    fn installer_creation_errors_are_returned_by_tasks() {
        let directory = tempdir().unwrap();
        let mut context = bake::Registry::new().context(directory.path());
        assert!(list(&mut context, None).is_err());
        assert!(show(&mut context, "provider".to_owned(), "guide.md".to_owned()).is_err());
        assert!(install(&mut context, None).is_err());
    }
}
