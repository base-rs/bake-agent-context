// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(test)]
use super::test_filesystem as fs;
use bake::{Error, Result};
use serde_yaml_ng::{Mapping, Value};
use socketry_markdown::{ParseOptions, mdast::Node, to_mdast};
#[cfg(not(test))]
use std::fs;
use std::path::{Path, PathBuf};

/// A Markdown source and its parsed YAML front matter.
#[derive(Clone, Debug)]
pub(crate) struct ContextDocument {
    pub(crate) path: PathBuf,
    pub(crate) content: String,
    pub(crate) frontmatter: Mapping,
    node: Option<Node>,
    body_start: usize,
}

impl ContextDocument {
    /// Read and parse a context document.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .map_err(|error| Error::new(format!("cannot read {}: {error}", path.display())))?;
        Self::parse(content, path)
    }

    /// Interpret YAML while leaving other front matter languages opaque.
    pub(crate) fn parse(content: String, path: &Path) -> Result<Self> {
        // Let the Markdown parser identify front matter and its source boundary:
        let mut options = ParseOptions::default();
        options.constructs.frontmatter = true;
        let document = to_mdast(&content, &options)
            .expect("Markdown parsing without MDX support is infallible");
        let first = document.children().and_then(|children| children.first());
        let mut frontmatter = Mapping::new();
        let mut node = None;
        let mut body_start = 0;

        // Decode the YAML node, retaining the original body bytes:
        if let Some(Node::Frontmatter(front)) = first {
            body_start = front
                .position
                .as_ref()
                .expect("parsed nodes have positions")
                .end
                .offset;
            let remaining = &content[body_start..];
            body_start += if remaining.starts_with("\r\n") {
                2
            } else if remaining.starts_with(['\r', '\n']) {
                1
            } else {
                0
            };
            if front.language() == Some("yaml") {
                let value: Value = serde_yaml_ng::from_str(&front.value).map_err(|error| {
                    Error::new(format!(
                        "invalid YAML front matter in {}: {error}",
                        path.display()
                    ))
                })?;
                frontmatter = match value {
                    Value::Null => Mapping::new(),
                    Value::Mapping(mapping) => mapping,
                    _ => {
                        return Err(Error::new(format!(
                            "context front matter must be a mapping: {}",
                            path.display()
                        )));
                    }
                };
            }
            node = first.cloned();
        }

        Ok(Self {
            path: path.to_owned(),
            content,
            frontmatter,
            node,
            body_start,
        })
    }

    /// Return the extensible metadata mapping, when one is present.
    pub(crate) fn metadata(&self) -> Option<&Mapping> {
        self.frontmatter.get("metadata").and_then(Value::as_mapping)
    }

    /// Whether the document declares a skill.
    pub(crate) fn is_skill(&self) -> bool {
        self.metadata()
            .and_then(|metadata| metadata.get("type"))
            .and_then(Value::as_str)
            == Some("skill")
    }

    /// Write the source that was loaded and validated.
    pub(crate) fn write(&self, path: &Path) -> Result<()> {
        fs::write(path, &self.content)
            .map_err(|error| Error::new(format!("cannot write {}: {error}", path.display())))
    }

    /// Serialize edited front matter through its AST node and retain the source body.
    pub(crate) fn render(&self, frontmatter: Mapping) -> String {
        let Some(Node::Frontmatter(mut node)) = self.node.clone() else {
            unreachable!("skill documents have a parsed front matter node");
        };
        node.value =
            serde_yaml_ng::to_string(&frontmatter).expect("parsed YAML mappings are serializable");
        let mut output = Node::Frontmatter(node).to_markdown();
        output.push_str(&self.content[self.body_start..]);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_native_frontmatter_and_preserves_the_body_bytes() {
        let fixture = include_str!("../tests/fixtures/SKILL.md");
        for ending in ["\n", "\r\n"] {
            let source = fixture.replace('\n', ending);
            let document =
                ContextDocument::parse(source.clone(), Path::new("workflow.md")).unwrap();
            assert!(document.is_skill());
            let body = &source[document.body_start..];
            let mut metadata = document.frontmatter.clone();
            metadata.insert("license".into(), "Apache-2.0".into());
            let output = document.render(metadata);
            let installed = ContextDocument::parse(output, Path::new("SKILL.md")).unwrap();
            assert_eq!(&installed.content[installed.body_start..], body);
            assert_eq!(
                installed.frontmatter["license"].as_str(),
                Some("Apache-2.0")
            );
            assert_eq!(
                installed.metadata().unwrap()["author"].as_str(),
                Some("Provider")
            );
        }
    }

    #[test]
    fn interprets_yaml_nodes_and_leaves_other_languages_opaque() {
        for (source, skill) in [
            ("--- yaml\nmetadata: {type: skill}\n---\nBody", true),
            ("``` yaml\nmetadata: {type: skill}\n```\nBody", true),
            ("--- json\nthis is not YAML [\n---\nBody", false),
            ("--- yml\nthis is not YAML [\n---\nBody", false),
            ("+++\nthis is TOML\n+++\nBody", false),
            ("---\n---", false),
            ("---\nmetadata: null\n---\n", false),
            ("---\nmetadata: {type: null}\n---\n", false),
            ("---\nmetadata: {type: guide}\n---\n", false),
            ("---\nmetadata: {type: false}\n---\n", false),
            ("---\ntype: skill\nmetadata: unrelated\n---\n", false),
            ("# Guide\n\n---\n", false),
        ] {
            let document =
                ContextDocument::parse(source.to_owned(), Path::new("guide.md")).unwrap();
            assert_eq!(document.is_skill(), skill, "{source}");
            if skill {
                assert!(
                    document
                        .render(document.frontmatter.clone())
                        .ends_with("Body")
                );
            }
        }
        for source in ["---\n[]\n---\n", "---\n[invalid\n---\n"] {
            assert!(
                ContextDocument::parse(source.to_owned(), Path::new("broken.md"))
                    .unwrap_err()
                    .to_string()
                    .contains("broken.md")
            );
        }
    }
}
