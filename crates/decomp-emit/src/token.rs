//! Owned, language-neutral lexical tokens.

/// A backend-defined literal category.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LitKind(String);

impl LitKind {
    /// Creates a literal category from backend-owned profile data.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the category name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A backend-defined comment category.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommentKind(String);

impl CommentKind {
    /// Creates a comment category from backend-owned profile data.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the category name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An owned token ready to be transformed into a document.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Token {
    /// A reserved language word.
    Keyword(String),
    /// An identifier spelling.
    Ident(String),
    /// A literal spelling and its backend-defined category.
    Literal {
        /// The backend-defined literal category.
        kind: LitKind,
        /// The exact spelling to emit.
        text: String,
    },
    /// An operator spelling.
    Operator(String),
    /// A punctuation spelling.
    Punct(String),
    /// A comment spelling and its backend-defined category.
    Comment {
        /// The backend-defined comment category.
        kind: CommentKind,
        /// The exact spelling to emit.
        text: String,
    },
    /// A logical line break.
    Newline,
}

impl Token {
    /// Returns emitted text for textual tokens and `None` for a newline.
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Keyword(text)
            | Self::Ident(text)
            | Self::Operator(text)
            | Self::Punct(text)
            | Self::Literal { text, .. }
            | Self::Comment { text, .. } => Some(text),
            Self::Newline => None,
        }
    }
}
