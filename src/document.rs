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
#[cfg(not(test))]
use std::fs;
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

/// A YAML mapping that retains its authored key order.
pub(crate) type Mapping = yaml_rust2::yaml::Hash;

/// Access YAML mapping entries by their string keys.
pub(crate) trait MappingFields {
    /// Return the value stored under a string key.
    fn field(&self, name: &str) -> Option<&Yaml>;

    /// Return the value stored under a string key when it is a string.
    fn string_field(&self, name: &str) -> Option<&str> {
        self.field(name).and_then(Yaml::as_str)
    }

    /// Store a value under a string key, keeping the position of an existing entry.
    fn insert_field(&mut self, name: &str, value: Yaml);
}

impl MappingFields for Mapping {
    fn field(&self, name: &str) -> Option<&Yaml> {
        self.get(&Yaml::String(name.to_owned()))
    }

    fn insert_field(&mut self, name: &str, value: Yaml) {
        // Unlike `insert`, `replace` does not move an existing entry to the end:
        self.replace(Yaml::String(name.to_owned()), value);
    }
}

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
                frontmatter = decode_frontmatter(document.frontmatter_value(id).unwrap(), path)?;
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
        self.frontmatter.field("metadata").and_then(Yaml::as_hash)
    }

    /// Whether the document declares a skill.
    pub(crate) fn is_skill(&self) -> bool {
        self.metadata()
            .and_then(|metadata| metadata.string_field("type"))
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
                    value: Text::from(encode_frontmatter(frontmatter)),
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

/// Decode one YAML document, treating absent and null front matter as an empty mapping.
fn decode_frontmatter(source: &str, path: &Path) -> Result<Mapping> {
    let invalid = |reason: &dyn std::fmt::Display| {
        Error::new(format!(
            "invalid YAML front matter in {}: {reason}",
            path.display()
        ))
    };
    let mut documents = YamlLoader::load_from_str(source).map_err(|error| invalid(&error))?;
    if documents.len() > 1 {
        return Err(invalid(&"more than one YAML document is not supported"));
    }
    match documents.pop() {
        None | Some(Yaml::Null) => Ok(Mapping::new()),
        Some(Yaml::Hash(mapping)) => Ok(mapping),
        Some(_) => Err(Error::new(format!(
            "context front matter must be a mapping: {}",
            path.display()
        ))),
    }
}

/// Encode a mapping as front matter content, which has no document start marker.
fn encode_frontmatter(frontmatter: Mapping) -> String {
    // Prefer literal blocks for multiline strings, but they cannot represent multiline keys:
    let document = Yaml::Hash(frontmatter);
    let mut output = emit_yaml(&document, true);
    if YamlLoader::load_from_str(&output).ok().as_deref() != Some(std::slice::from_ref(&document)) {
        output = emit_yaml(&document, false);
    }
    let mut content = output
        .strip_prefix("---\n")
        .expect("the YAML emitter starts each document with a marker")
        .to_owned();
    content.push('\n');
    content
}

fn emit_yaml(document: &Yaml, multiline_strings: bool) -> String {
    let mut output = String::new();
    let mut emitter = YamlEmitter::new(&mut output);
    emitter.multiline_strings(multiline_strings);
    emitter
        .dump(document)
        .expect("YAML mappings can be written to a string");
    output
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
            metadata.insert_field("license", Yaml::String("Apache-2.0".to_owned()));
            let output = document.render(metadata);
            let installed = Document::parse(output, Path::new("SKILL.md")).unwrap();
            assert_eq!(&installed.content()[installed.body_start..], body);
            assert_eq!(
                installed.frontmatter.string_field("license"),
                Some("Apache-2.0")
            );
            assert_eq!(
                installed.metadata().unwrap().string_field("author"),
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

    fn parse_error(source: &str) -> String {
        Document::parse(source.to_owned(), Path::new("broken.md"))
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn decodes_a_single_yaml_document_as_a_mapping() {
        for source in ["---\n---\n", "---\n# Comment only.\n---\n", "---\n~\n---\n"] {
            let document = Document::parse(source.to_owned(), Path::new("guide.md")).unwrap();
            assert!(document.frontmatter.is_empty(), "{source}");
        }
        assert_eq!(
            parse_error("---\n- item\n---\n"),
            "context front matter must be a mapping: broken.md"
        );
        assert!(
            parse_error("---\n[invalid\n---\n")
                .starts_with("invalid YAML front matter in broken.md: ")
        );
        assert!(parse_error("---\nname: a\nname: b\n---\n").contains("duplicated key"));

        // A code fence can hold a document marker, which would begin a second document:
        assert_eq!(
            parse_error("``` yaml\nname: first\n---\nname: second\n```\nBody"),
            "invalid YAML front matter in broken.md: more than one YAML document is not supported"
        );
    }

    #[test]
    fn renders_one_front_matter_block_in_mapping_order() {
        let source = "---\nzeta: 1\nalpha: '1.0'\ndescription: >\n  Folded\n  text.\nlist: [a, b]\n---\nBody";
        let document = Document::parse(source.to_owned(), Path::new("guide.md")).unwrap();
        let mut frontmatter = document.frontmatter.clone();
        frontmatter.insert_field("zeta", Yaml::Integer(2));
        frontmatter.insert_field("name", Yaml::String("guide".to_owned()));
        assert_eq!(
            document.render(frontmatter),
            "---\nzeta: 2\nalpha: \"1.0\"\ndescription: |\n  Folded text.\nlist:\n  - a\n  - b\nname: guide\n---\nBody"
        );
        assert_eq!(document.render(Mapping::new()), "---\n{}\n---\nBody");
    }

    #[test]
    fn quotes_multiline_keys_that_literal_blocks_cannot_represent() {
        let source = "---\n\"first\\nsecond\": value\n---\nBody";
        let document = Document::parse(source.to_owned(), Path::new("guide.md")).unwrap();
        let output = document.render(document.frontmatter.clone());
        assert_eq!(output, "---\n\"first\\nsecond\": value\n---\nBody");
        let installed = Document::parse(output, Path::new("SKILL.md")).unwrap();
        assert_eq!(installed.frontmatter, document.frontmatter);
    }
}
