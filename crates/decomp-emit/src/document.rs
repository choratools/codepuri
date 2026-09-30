//! Recursive document layout and rendering.
//!
//! Rendering deliberately uses a simple recursive model: groups preserve
//! composition boundaries but do not perform line-width fitting. Backends make
//! line breaks explicit, which keeps output and source coordinates predictable.

use crate::error::EmitError;
use crate::sourcemap::{Mapping, Position, ProvenanceId, SourceMap};

/// A composable text layout tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Doc {
    /// No output.
    Empty,
    /// Verbatim text without line breaks.
    Text(String),
    /// A logical line break.
    Line,
    /// A composition boundary retained for future layout strategies.
    Group(Box<Self>),
    /// Content rendered one indentation level deeper after line breaks.
    Indent(Box<Self>),
    /// Documents rendered in order.
    Concat(Vec<Self>),
    /// Content associated with caller-owned provenance.
    Annotated {
        /// The opaque provenance identity.
        provenance: ProvenanceId,
        /// The annotated content.
        content: Box<Self>,
    },
}

impl Doc {
    /// Creates a text node.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// Creates a logical line break.
    pub const fn line() -> Self {
        Self::Line
    }

    /// Groups a document without changing its explicit line breaks.
    pub fn group(content: Self) -> Self {
        Self::Group(Box::new(content))
    }

    /// Increases indentation for lines begun inside the document.
    pub fn indent(content: Self) -> Self {
        Self::Indent(Box::new(content))
    }

    /// Concatenates documents in iteration order.
    pub fn concat(documents: impl IntoIterator<Item = Self>) -> Self {
        Self::Concat(documents.into_iter().collect())
    }

    /// Associates emitted text with an opaque provenance identity.
    pub fn with_provenance(self, provenance: ProvenanceId) -> Self {
        Self::Annotated {
            provenance,
            content: Box::new(self),
        }
    }
}

/// The text and source map produced by one render.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderedDocument {
    /// Final emitted text.
    pub text: String,
    /// Provenance mappings for annotated text.
    pub source_map: SourceMap,
}

/// A deterministic recursive renderer with configurable indentation width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Renderer {
    indent_width: usize,
}

impl Renderer {
    /// Creates a renderer whose indentation level uses `indent_width` spaces.
    pub const fn new(indent_width: usize) -> Self {
        Self { indent_width }
    }

    /// Renders a document and records output-coordinate provenance.
    ///
    /// # Errors
    /// Returns an error for embedded newlines or coordinate overflow.
    pub fn render(self, document: &Doc) -> Result<RenderedDocument, EmitError> {
        let mut state = RenderState::new(self.indent_width);
        state.render(document, None)?;
        Ok(RenderedDocument {
            text: state.output,
            source_map: state.source_map,
        })
    }
}

struct RenderState {
    output: String,
    source_map: SourceMap,
    indent_width: usize,
    depth: usize,
    position: Position,
    line_start: bool,
}

impl RenderState {
    fn new(indent_width: usize) -> Self {
        Self {
            output: String::new(),
            source_map: SourceMap::default(),
            indent_width,
            depth: 0,
            position: Position::new(0, 0),
            line_start: true,
        }
    }

    fn render(
        &mut self,
        document: &Doc,
        provenance: Option<ProvenanceId>,
    ) -> Result<(), EmitError> {
        match document {
            Doc::Empty => Ok(()),
            Doc::Text(text) => self.write_text(text, provenance),
            Doc::Line => self.write_line(),
            Doc::Group(content) => self.render(content, provenance),
            Doc::Indent(content) => {
                self.depth = self
                    .depth
                    .checked_add(1)
                    .ok_or(EmitError::PositionOverflow)?;
                let result = self.render(content, provenance);
                self.depth -= 1;
                result
            }
            Doc::Concat(documents) => {
                for child in documents {
                    self.render(child, provenance)?;
                }
                Ok(())
            }
            Doc::Annotated {
                provenance: nested,
                content,
            } => self.render(content, Some(*nested)),
        }
    }

    fn write_text(
        &mut self,
        text: &str,
        provenance: Option<ProvenanceId>,
    ) -> Result<(), EmitError> {
        if text.contains(['\n', '\r']) {
            return Err(EmitError::EmbeddedNewline);
        }
        if text.is_empty() {
            return Ok(());
        }
        self.write_indent()?;
        let start = self.position;
        let width = text.chars().count();
        self.position.column = self
            .position
            .column
            .checked_add(width)
            .ok_or(EmitError::PositionOverflow)?;
        self.output.push_str(text);
        if let Some(identity) = provenance {
            self.source_map
                .push(Mapping::new(start, self.position, identity));
        }
        Ok(())
    }

    fn write_indent(&mut self) -> Result<(), EmitError> {
        if !self.line_start {
            return Ok(());
        }
        let width = self
            .depth
            .checked_mul(self.indent_width)
            .ok_or(EmitError::PositionOverflow)?;
        self.position.column = width;
        self.output.extend(std::iter::repeat_n(' ', width));
        self.line_start = false;
        Ok(())
    }

    fn write_line(&mut self) -> Result<(), EmitError> {
        self.output.push('\n');
        self.position.line = self
            .position
            .line
            .checked_add(1)
            .ok_or(EmitError::PositionOverflow)?;
        self.position.column = 0;
        self.line_start = true;
        Ok(())
    }
}
