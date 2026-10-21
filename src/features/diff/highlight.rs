//! Syntax colours for diff lines.
//!
//! A small lexical tokenizer fills them: comments, strings, numbers,
//! keywords, types and calls, picked by the file's extension. It reads line by
//! line and carries only whether a block comment is open, so it works on a
//! whole file and on a hunk's fragments alike. A document is a list of
//! coloured spans per line, and [`runs_for_line`] turns a line's spans into
//! what GPUI paints.

use std::ops::Range;

use gpui_kit::{Font, Hsla, TextRun};

use crate::ui::theme::{Syntax, Theme};

/// What a stretch of code is, for colouring. Few kinds on purpose: a diff is
/// read for what changed, and at this size many colours read as noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    Comment,
    Keyword,
    String,
    Number,
    Type,
    Function,
    Punctuation,
}

impl HighlightKind {
    fn color(self, syntax: &Syntax) -> Hsla {
        match self {
            Self::Comment => syntax.comment,
            Self::Keyword => syntax.keyword,
            Self::String => syntax.string,
            Self::Number => syntax.number,
            Self::Type => syntax.type_name,
            Self::Function => syntax.function,
            Self::Punctuation => syntax.punctuation,
        }
    }
}

/// A coloured stretch of a line, in bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

/// A file's spans, one list per line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HighlightedDocument {
    pub lines: Vec<Vec<HighlightSpan>>,
}

/// How one language is lexed. Only what a line-local scan can tell apart.
#[derive(Debug)]
pub struct Language {
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    /// Characters that open a string closed by the same character.
    quotes: &'static [char],
    /// `'` opens a character literal only when it closes within a few
    /// characters; otherwise it is a lifetime or a label (Rust).
    char_literals: bool,
    keywords: &'static [&'static str],
}

const C_LIKE: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "class",
    "namespace",
    "template",
    "typename",
    "public",
    "private",
    "protected",
    "virtual",
    "override",
    "new",
    "delete",
    "this",
    "true",
    "false",
    "nullptr",
    "NULL",
    "using",
    "include",
    "define",
    "ifdef",
    "ifndef",
    "endif",
    "bool",
    "try",
    "catch",
    "throw",
    "constexpr",
    "noexcept",
    "operator",
    "friend",
];

const RUST: Language = Language {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"'],
    char_literals: true,
    keywords: &[
        "as",
        "async",
        "await",
        "break",
        "const",
        "continue",
        "crate",
        "dyn",
        "else",
        "enum",
        "extern",
        "false",
        "fn",
        "for",
        "if",
        "impl",
        "in",
        "let",
        "loop",
        "match",
        "mod",
        "move",
        "mut",
        "pub",
        "ref",
        "return",
        "self",
        "Self",
        "static",
        "struct",
        "super",
        "trait",
        "true",
        "type",
        "unsafe",
        "use",
        "where",
        "while",
        "yield",
        "macro_rules",
    ],
};

const C: Language = Language {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    char_literals: false,
    keywords: C_LIKE,
};

const JS: Language = Language {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\'', '`'],
    char_literals: false,
    keywords: &[
        "async",
        "await",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "default",
        "delete",
        "do",
        "else",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "if",
        "import",
        "in",
        "instanceof",
        "let",
        "new",
        "null",
        "of",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "undefined",
        "var",
        "void",
        "while",
        "yield",
        "interface",
        "type",
        "enum",
        "implements",
        "readonly",
        "private",
        "public",
        "protected",
        "abstract",
        "as",
        "declare",
        "keyof",
        "namespace",
    ],
};

const JVM: Language = Language {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[
        "abstract",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "default",
        "do",
        "else",
        "enum",
        "extends",
        "final",
        "finally",
        "for",
        "if",
        "implements",
        "import",
        "instanceof",
        "interface",
        "new",
        "null",
        "package",
        "private",
        "protected",
        "public",
        "return",
        "static",
        "super",
        "switch",
        "this",
        "throw",
        "throws",
        "true",
        "false",
        "try",
        "void",
        "while",
        "val",
        "var",
        "fun",
        "object",
        "when",
        "is",
        "in",
        "override",
        "data",
        "sealed",
        "open",
        "internal",
        "func",
        "let",
        "struct",
        "protocol",
        "extension",
        "guard",
        "self",
        "nil",
        "using",
        "namespace",
        "async",
        "await",
        "readonly",
        "get",
        "set",
        "init",
        "import",
        "export",
        "library",
        "late",
        "required",
        "int",
        "long",
        "double",
        "float",
        "boolean",
        "char",
        "byte",
        "short",
        "string",
        "bool",
    ],
};

const GO: Language = Language {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\'', '`'],
    char_literals: false,
    keywords: &[
        "break",
        "case",
        "chan",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "fallthrough",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "interface",
        "map",
        "package",
        "range",
        "return",
        "select",
        "struct",
        "switch",
        "type",
        "var",
        "true",
        "false",
        "nil",
        "iota",
    ],
};

const PYTHON: Language = Language {
    line_comments: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[
        "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del",
        "elif", "else", "except", "False", "finally", "for", "from", "global", "if", "import",
        "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True",
        "try", "while", "with", "yield", "self", "match", "case",
    ],
};

const RUBY: Language = Language {
    line_comments: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[
        "alias",
        "and",
        "begin",
        "break",
        "case",
        "class",
        "def",
        "do",
        "else",
        "elsif",
        "end",
        "ensure",
        "false",
        "for",
        "if",
        "in",
        "module",
        "next",
        "nil",
        "not",
        "or",
        "redo",
        "rescue",
        "retry",
        "return",
        "self",
        "super",
        "then",
        "true",
        "undef",
        "unless",
        "until",
        "when",
        "while",
        "yield",
        "require",
        "attr_reader",
        "attr_accessor",
    ],
};

const SHELL: Language = Language {
    line_comments: &["#"],
    block_comment: None,
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
        "in", "function", "return", "local", "export", "readonly", "set", "unset", "echo", "exit",
        "source", "true", "false",
    ],
};

/// TOML, YAML, INI and friends: comments, strings, numbers.
const CONFIG: Language = Language {
    line_comments: &["#", ";"],
    block_comment: None,
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &["true", "false", "null", "yes", "no", "on", "off"],
};

const JSON: Language = Language {
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    quotes: &['"'],
    char_literals: false,
    keywords: &["true", "false", "null"],
};

const CSS: Language = Language {
    line_comments: &[],
    block_comment: Some(("/*", "*/")),
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[
        "important",
        "media",
        "import",
        "keyframes",
        "from",
        "to",
        "supports",
    ],
};

const MARKUP: Language = Language {
    line_comments: &[],
    block_comment: Some(("<!--", "-->")),
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[],
};

const SQL: Language = Language {
    line_comments: &["--"],
    block_comment: Some(("/*", "*/")),
    quotes: &['\''],
    char_literals: false,
    keywords: &[
        "select",
        "from",
        "where",
        "insert",
        "into",
        "values",
        "update",
        "set",
        "delete",
        "create",
        "table",
        "drop",
        "alter",
        "index",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "on",
        "and",
        "or",
        "not",
        "null",
        "as",
        "order",
        "by",
        "group",
        "having",
        "limit",
        "primary",
        "key",
        "foreign",
        "references",
        "SELECT",
        "FROM",
        "WHERE",
        "INSERT",
        "INTO",
        "VALUES",
        "UPDATE",
        "SET",
        "DELETE",
        "CREATE",
        "TABLE",
        "DROP",
        "ALTER",
        "INDEX",
        "JOIN",
        "LEFT",
        "RIGHT",
        "INNER",
        "OUTER",
        "ON",
        "AND",
        "OR",
        "NOT",
        "NULL",
        "AS",
        "ORDER",
        "BY",
        "GROUP",
        "HAVING",
        "LIMIT",
        "PRIMARY",
        "KEY",
        "FOREIGN",
        "REFERENCES",
    ],
};

const LUA: Language = Language {
    line_comments: &["--"],
    block_comment: None,
    quotes: &['"', '\''],
    char_literals: false,
    keywords: &[
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
        "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ],
};

impl Language {
    /// The language of `path`, by extension or well-known name. `None` for
    /// anything else, which then draws plain.
    pub fn of(path: &str) -> Option<&'static Language> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let extension = name.rsplit_once('.').map_or("", |(_, ext)| ext);
        let language = match (name, extension.to_ascii_lowercase().as_str()) {
            ("Makefile" | "Dockerfile" | ".bashrc" | ".zshrc" | ".profile", _) => &SHELL,
            (".gitignore" | ".gitattributes" | ".editorconfig", _) => &CONFIG,
            (_, "rs") => &RUST,
            (_, "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "m" | "mm") => &C,
            (_, "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "vue" | "svelte") => {
                &JS
            }
            (
                _,
                "java" | "kt" | "kts" | "scala" | "cs" | "swift" | "dart" | "groovy" | "gradle",
            ) => &JVM,
            (_, "go") => &GO,
            (_, "py" | "pyi") => &PYTHON,
            (_, "rb" | "rake" | "gemspec") => &RUBY,
            (_, "sh" | "bash" | "zsh" | "fish" | "ps1") => &SHELL,
            (_, "toml" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "properties" | "env") => &CONFIG,
            (_, "json" | "jsonc" | "json5") => &JSON,
            (_, "css" | "scss" | "sass" | "less") => &CSS,
            (_, "html" | "htm" | "xml" | "svg" | "xhtml" | "plist") => &MARKUP,
            (_, "sql") => &SQL,
            (_, "lua") => &LUA,
            _ => return None,
        };
        Some(language)
    }
}

/// Tokenizes consecutive lines of one language. The only state carried from
/// line to line is an open block comment.
#[derive(Debug)]
pub struct Highlighter {
    language: &'static Language,
    in_block: bool,
}

impl Highlighter {
    pub fn new(language: &'static Language) -> Self {
        Self {
            language,
            in_block: false,
        }
    }

    /// Forget an open block comment: the next line is not where this one
    /// left off (a new hunk).
    pub fn reset(&mut self) {
        self.in_block = false;
    }

    /// The spans of the next line, in order and not overlapping.
    pub fn line(&mut self, text: &str) -> Vec<HighlightSpan> {
        let language = self.language;
        let bytes = text.as_bytes();
        let mut spans = Vec::new();
        let mut push = |range: Range<usize>, kind| {
            if !range.is_empty() {
                spans.push(HighlightSpan { range, kind });
            }
        };
        let mut at = 0;
        if self.in_block {
            let (_, close) = language
                .block_comment
                .expect("only set with a block comment");
            match text.find(close) {
                Some(end) => {
                    push(0..end + close.len(), HighlightKind::Comment);
                    at = end + close.len();
                    self.in_block = false;
                }
                None => {
                    push(0..text.len(), HighlightKind::Comment);
                    return spans;
                }
            }
        }
        while at < text.len() {
            let rest = &text[at..];
            if language.line_comments.iter().any(|c| rest.starts_with(c))
                && is_comment_start(text, at)
            {
                push(at..text.len(), HighlightKind::Comment);
                break;
            }
            if let Some((open, close)) = language.block_comment
                && rest.starts_with(open)
            {
                match rest[open.len()..].find(close) {
                    Some(end) => {
                        let end = at + open.len() + end + close.len();
                        push(at..end, HighlightKind::Comment);
                        at = end;
                    }
                    None => {
                        push(at..text.len(), HighlightKind::Comment);
                        self.in_block = true;
                        break;
                    }
                }
                continue;
            }
            let Some(c) = rest.chars().next() else { break };
            if language.quotes.contains(&c) {
                let end = string_end(text, at, c);
                push(at..end, HighlightKind::String);
                at = end;
                continue;
            }
            if c == '\'' && language.char_literals {
                if let Some(end) = char_literal_end(text, at) {
                    push(at..end, HighlightKind::String);
                    at = end;
                } else {
                    at += 1;
                }
                continue;
            }
            if c.is_ascii_digit() {
                let end = word_end(text, at);
                push(at..end, HighlightKind::Number);
                at = end;
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let end = word_end(text, at);
                let word = &text[at..end];
                let next = text[end..].trim_start();
                let kind = if language.keywords.contains(&word) {
                    Some(HighlightKind::Keyword)
                } else if next.starts_with('(')
                    || (language.char_literals && bytes.get(end) == Some(&b'!'))
                {
                    Some(HighlightKind::Function)
                } else if c.is_uppercase() {
                    Some(HighlightKind::Type)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    push(at..end, kind);
                }
                at = end;
                continue;
            }
            at += c.len_utf8();
        }
        spans
    }
}

/// `#` and `;` open a comment only at the start of a word: `a#b` in a URL
/// or `x;` at the end of a statement are not comments.
fn is_comment_start(text: &str, at: usize) -> bool {
    let c = text[at..].chars().next();
    if !matches!(c, Some('#' | ';')) {
        return true;
    }
    if c == Some(';') && at > 0 {
        return false;
    }
    text[..at]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace)
}

/// Where the string opened by `quote` at `at` ends: past its closing quote,
/// or at the end of the line.
fn string_end(text: &str, at: usize, quote: char) -> usize {
    let mut escaped = false;
    for (ix, c) in text[at + quote.len_utf8()..].char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            _ if c == quote => return at + quote.len_utf8() + ix + c.len_utf8(),
            _ => {}
        }
    }
    text.len()
}

/// `'x'`, `'\n'`, `'\u{1F600}'`: a character literal, or `None` for a
/// lifetime or a label.
fn char_literal_end(text: &str, at: usize) -> Option<usize> {
    let rest = &text[at + 1..];
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first == '\\' {
        // Past the backslash and the escaped character, which may itself
        // be a quote.
        let close = rest.get(2..)?.find('\'')?;
        return (close <= 10).then_some(at + 1 + 2 + close + 1);
    }
    let (ix, second) = chars.next()?;
    (second == '\'').then_some(at + 1 + ix + 1)
}

fn word_end(text: &str, at: usize) -> usize {
    text[at..]
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map_or(text.len(), |(ix, _)| at + ix)
}

/// A whole file's spans, or `None` when its language is not known.
pub fn highlight_document<'a>(
    path: &str,
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<HighlightedDocument> {
    let mut highlighter = Highlighter::new(Language::of(path)?);
    Some(HighlightedDocument {
        lines: lines
            .into_iter()
            .map(|line| highlighter.line(line))
            .collect(),
    })
}

/// The runs that paint `line`: its spans in their fixed colours, and the rest
/// in the palette's plain colour. Spans are taken in order and must not
/// overlap.
pub fn runs_for_line(
    line: &str,
    spans: &[HighlightSpan],
    mono: &Font,
    theme: &Theme,
) -> Vec<TextRun> {
    let plain = theme.syntax.plain;
    let run = |len: usize, color: Hsla| TextRun {
        len,
        font: mono.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let mut runs = Vec::with_capacity(spans.len() * 2 + 1);
    let mut done = 0;
    for span in spans {
        runs.push(run(span.range.start.saturating_sub(done), plain));
        runs.push(run(span.range.len(), span.kind.color(&theme.syntax)));
        done = span.range.end;
    }
    runs.push(run(line.len().saturating_sub(done), plain));
    runs.retain(|run| run.len > 0);
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint(line: &str, spans: &[HighlightSpan]) -> (Vec<usize>, Vec<Hsla>, Theme) {
        let theme = Theme::dark();
        let runs = runs_for_line(line, spans, &gpui_kit::font("monospace"), &theme);
        (
            runs.iter().map(|r| r.len).collect(),
            runs.iter().map(|r| r.color).collect(),
            theme,
        )
    }

    #[test]
    fn a_plain_line_is_one_run() {
        let (lengths, colors, theme) = paint("x = 1", &[]);
        assert_eq!(lengths, [5]);
        assert_eq!(colors, [theme.syntax.plain]);
    }

    #[test]
    fn spans_are_coloured_and_the_gaps_left_plain() {
        let spans = [
            HighlightSpan {
                range: 0..2,
                kind: HighlightKind::Keyword,
            },
            HighlightSpan {
                range: 6..8,
                kind: HighlightKind::String,
            },
        ];
        let (lengths, colors, theme) = paint("if x \"y\" {}", &spans);
        assert_eq!(lengths, [2, 4, 2, 3]);
        assert_eq!(lengths.iter().sum::<usize>(), 11);
        assert_eq!(colors[0], HighlightKind::Keyword.color(&theme.syntax));
        assert_eq!(colors[1], theme.syntax.plain);
        assert_eq!(colors[2], HighlightKind::String.color(&theme.syntax));
    }

    #[test]
    fn the_palette_does_not_follow_the_accent() {
        // The same token keeps its colour whatever accent the chrome uses.
        let default = Theme::dark();
        let other = {
            let mut theme = Theme::dark();
            theme.accent = gpui_kit::hsla(0.3, 0.8, 0.5, 1.0);
            theme
        };
        for kind in [
            HighlightKind::Comment,
            HighlightKind::Keyword,
            HighlightKind::String,
            HighlightKind::Number,
            HighlightKind::Type,
            HighlightKind::Function,
        ] {
            assert_eq!(kind.color(&default.syntax), kind.color(&other.syntax));
        }
    }

    #[test]
    fn dark_and_light_read_code_in_their_own_colours() {
        let dark = Theme::dark().syntax;
        let light = Theme::light().syntax;
        assert_ne!(dark.plain, light.plain);
        assert_ne!(dark.keyword, light.keyword);
    }
}
