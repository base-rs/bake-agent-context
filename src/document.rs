// Released under the MIT License.
// Copyright, 2026, by Samuel Williams.

#[cfg(test)]
use super::test_filesystem as fs;
use bake::{Error, Result};
use base_markdown::{
    Document as MarkdownDocument, MarkdownOptions, NodeSyntax, ParseOptions,
    stream::{EventKind, Text},
    to_markdown_node,
};
use serde_yaml_ng::{Mapping, Value};
#[cfg(not(test))]
use std::fs;
use std::path::{Path, PathBuf};

/// A Markdown source and its parsed YAML front matter.
#[derive(Clone, Debug)]
pub(crate) struct Document {
    pub(crate) path: PathBuf,
    content: String,
    pub(crate) frontmatter: Mapping,
    syntax: Option<NodeSyntax<'static>>,
    body_start: usize,
}

impl Document {
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
        let document = MarkdownDocument::parse(&content, &options)
            .expect("the built-in Markdown parser produces a valid document");
        let first = document.children(document.root()).next();
        let mut frontmatter = Mapping::new();
        let mut syntax = None;
        let mut body_start = 0;

        // Decode the YAML node, retaining the original body bytes:
        if let Some((
            id,
            NodeSyntax::Frontmatter {
                info,
                fence,
                closing_fence_length,
            },
        )) = first.and_then(|id| document.syntax(id).map(|syntax| (id, syntax)))
        {
            body_start = document.node(id).unwrap().span.end;
            let remaining = &document.source()[body_start..];
            body_start += if remaining.starts_with("\r\n") {
                2
            } else if remaining.starts_with(['\r', '\n']) {
                1
            } else {
                0
            };
            let info = info.resolve(document.source()).unwrap();
            let language = info
                .split_ascii_whitespace()
                .next()
                .or(match fence.character {
                    '-' => Some("yaml"),
                    '+' => Some("toml"),
                    _ => None,
                });
            if language == Some("yaml") {
                let value: Value = serde_yaml_ng::from_str(document.frontmatter_value(id).unwrap())
                    .map_err(|error| {
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
            syntax = Some(NodeSyntax::Frontmatter {
                info: Text::from(info.to_owned()),
                fence,
                closing_fence_length,
            });
        }

        Ok(Self {
            path: path.to_owned(),
            content,
            frontmatter,
            syntax,
            body_start,
        })
    }

    pub(crate) fn content(&self) -> &str {
        &self.content
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
        fs::write(path, self.content())
            .map_err(|error| Error::new(format!("cannot write {}: {error}", path.display())))
    }

    /// Serialize edited front matter through its AST node and retain the source body.
    pub(crate) fn render(&self, frontmatter: Mapping) -> String {
        let mut document = MarkdownDocument::parse("", &ParseOptions::default()).unwrap();
        let node = document
            .append_child(
                document.root(),
                EventKind::Opaque {
                    value: Text::from(
                        serde_yaml_ng::to_string(&frontmatter)
                            .expect("parsed YAML mappings are serializable"),
                    ),
                    block: true,
                },
            )
            .unwrap();
        document.set_syntax(node, self.syntax.clone()).unwrap();
        let mut output = to_markdown_node(&document, node, &MarkdownOptions::default())
            .expect("edited frontmatter has valid Markdown syntax");
        output.push_str(&self.content()[self.body_start..]);
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
            let document = Document::parse(source.clone(), Path::new("workflow.md")).unwrap();
            assert!(document.is_skill());
            let body = &source[document.body_start..];
            let mut metadata = document.frontmatter.clone();
            metadata.insert("license".into(), "Apache-2.0".into());
            let output = document.render(metadata);
            let installed = Document::parse(output, Path::new("SKILL.md")).unwrap();
            assert_eq!(&installed.content()[installed.body_start..], body);
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
            ("---\nmetadata: []\n---\n", false),
            ("---\nmetadata: 42\n---\n", false),
            ("---\nmetadata: {type: null}\n---\n", false),
            ("---\nmetadata: {type: guide}\n---\n", false),
            ("---\nmetadata: {type: false}\n---\n", false),
            ("---\ntype: skill\nmetadata: unrelated\n---\n", false),
            ("# Guide\n\n---\n", false),
        ] {
            let document = Document::parse(source.to_owned(), Path::new("guide.md")).unwrap();
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
                Document::parse(source.to_owned(), Path::new("broken.md"))
                    .unwrap_err()
                    .to_string()
                    .contains("broken.md")
            );
        }
    }
}
