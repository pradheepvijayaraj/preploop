//! Safe FTS5 query compiler.
//!
//! # Purpose
//!
//! Never pass raw user text directly to SQLite `MATCH`.
//! Malformed characters such as `"`, `*`, `(`, `)`, `-`, `^`, `:`, `{`, `}`
//! or reserved operators `AND`, `OR`, `NOT`, `NEAR` cause syntax errors or
//! unintended query semantics if passed unescaped.
//!
//! `FtsQueryBuilder` sanitizes, tokenizes, and compiles user input into
//! safe, predictable FTS5 match expressions.

/// Options for compiling an FTS query.
#[derive(Debug, Clone)]
pub struct FtsQueryOptions {
    /// If true, appends a prefix wildcard `*` to the final token for
    /// search-as-you-type / prefix completion.
    pub enable_prefix_matching: bool,
    /// Minimum token length to append a prefix wildcard.
    pub min_prefix_len: usize,
}

impl Default for FtsQueryOptions {
    fn default() -> Self {
        Self {
            enable_prefix_matching: true,
            min_prefix_len: 2,
        }
    }
}

/// A compiled, safe FTS5 query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFtsQuery {
    raw_query: String,
    fts_expr: String,
    terms: Vec<String>,
}

impl CompiledFtsQuery {
    /// Returns the compiled FTS5 expression suitable for `MATCH ?`.
    pub fn as_fts_match(&self) -> &str {
        &self.fts_expr
    }

    /// Returns the sanitized extracted search terms.
    pub fn terms(&self) -> &[String] {
        &self.terms
    }

    /// Builds a broad OR expression for a second-pass recall search.
    ///
    /// The primary expression deliberately keeps FTS5's implicit AND
    /// semantics. When that produces too few candidates, this expression
    /// recovers documents containing any meaningful query term while BM25
    /// still favours documents covering the rarer terms.
    pub fn as_relaxed_fts_match(&self) -> Option<String> {
        let meaningful = self
            .terms
            .iter()
            .filter(|term| !is_relaxed_stop_word(term))
            .collect::<Vec<_>>();
        let selected = if meaningful.is_empty() {
            self.terms.iter().collect::<Vec<_>>()
        } else {
            meaningful
        };
        if selected.len() < 2 {
            return None;
        }
        let quoted = selected
            .into_iter()
            .map(|term| format!("\"{}\"", escape_quotes(term)))
            .collect::<Vec<_>>();
        let mut pairs = Vec::new();
        for left in 0..quoted.len() {
            for right in (left + 1)..quoted.len() {
                pairs.push(format!("({} {})", quoted[left], quoted[right]));
            }
        }
        Some(pairs.join(" OR "))
    }

    /// Whether this query resulted in zero searchable terms.
    pub fn is_empty(&self) -> bool {
        self.fts_expr.is_empty()
    }
}

fn is_relaxed_stop_word(term: &str) -> bool {
    matches!(
        term.to_lowercase().as_str(),
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "for"
            | "from"
            | "how"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "the"
            | "to"
            | "was"
            | "were"
            | "what"
            | "which"
            | "with"
    )
}

/// Compiles arbitrary user input into a safe SQLite FTS5 MATCH expression.
pub struct FtsQueryBuilder;

impl FtsQueryBuilder {
    /// Compiles `input` with default options.
    pub fn build(input: &str) -> Option<CompiledFtsQuery> {
        Self::build_with_options(input, &FtsQueryOptions::default())
    }

    /// Compiles `input` with specific options.
    pub fn build_with_options(input: &str, options: &FtsQueryOptions) -> Option<CompiledFtsQuery> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Tokenize into phrases and unquoted words
        let tokens = extract_tokens(trimmed);
        if tokens.is_empty() {
            return None;
        }

        let num_tokens = tokens.len();
        let mut fts_parts: Vec<String> = Vec::with_capacity(num_tokens);
        let mut clean_terms: Vec<String> = Vec::with_capacity(num_tokens);

        for (i, token) in tokens.iter().enumerate() {
            let is_last = i == num_tokens - 1;
            match token {
                Token::Phrase(phrase) => {
                    let sanitized = sanitize_string(phrase);
                    if !sanitized.is_empty() {
                        fts_parts.push(format!("\"{}\"", escape_quotes(&sanitized)));
                        clean_terms.push(sanitized);
                    }
                }
                Token::Word(word) => {
                    let sanitized = sanitize_word(word);
                    if sanitized.is_empty() {
                        continue;
                    }

                    // Check if reserved FTS keyword
                    let is_reserved = matches!(
                        sanitized.to_uppercase().as_str(),
                        "AND" | "OR" | "NOT" | "NEAR"
                    );

                    clean_terms.push(sanitized.clone());

                    if is_last
                        && options.enable_prefix_matching
                        && sanitized.chars().count() >= options.min_prefix_len
                        && !sanitized
                            .chars()
                            .all(|character| character.is_ascii_digit())
                        && !is_reserved
                    {
                        // Trailing token gets prefix match: "term"*
                        fts_parts.push(format!("\"{}\"*", escape_quotes(&sanitized)));
                    } else {
                        // Standard term enclosed in double quotes for exact token matching & safety
                        fts_parts.push(format!("\"{}\"", escape_quotes(&sanitized)));
                    }
                }
            }
        }

        if fts_parts.is_empty() {
            return None;
        }

        let fts_expr = fts_parts.join(" ");
        Some(CompiledFtsQuery {
            raw_query: trimmed.to_string(),
            fts_expr,
            terms: clean_terms,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal tokenization & sanitization
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Phrase(String),
    Word(String),
}

fn extract_tokens(input: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut in_quotes = false;
    let mut current = String::new();

    for ch in input.chars() {
        if ch == '"' {
            if in_quotes {
                // End of quoted phrase
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    tokens.push(Token::Phrase(trimmed.to_string()));
                }
                current.clear();
                in_quotes = false;
            } else {
                // Start of quoted phrase
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    for w in trimmed.split_whitespace() {
                        tokens.push(Token::Word(w.to_string()));
                    }
                }
                current.clear();
                in_quotes = true;
            }
        } else {
            current.push(ch);
        }
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        if in_quotes {
            // Unclosed quote: treat remainder as phrase
            tokens.push(Token::Phrase(trimmed.to_string()));
        } else {
            for w in trimmed.split_whitespace() {
                tokens.push(Token::Word(w.to_string()));
            }
        }
    }

    tokens
}

fn sanitize_word(word: &str) -> String {
    // Keep alphanumeric and basic hyphen/underscore
    word.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect::<String>()
        .trim_matches(|c| c == '-' || c == '_')
        .to_string()
}

fn sanitize_string(s: &str) -> String {
    // Remove characters that disrupt FTS5 phrases
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '-' || *c == '_')
        .collect::<String>()
        .trim()
        .to_string()
}

fn escape_quotes(s: &str) -> String {
    s.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_and_whitespace() {
        assert_eq!(FtsQueryBuilder::build(""), None);
        assert_eq!(FtsQueryBuilder::build("   "), None);
        assert_eq!(FtsQueryBuilder::build("***"), None);
    }

    #[test]
    fn test_single_word_prefix() {
        let q = FtsQueryBuilder::build("article").unwrap();
        assert_eq!(q.as_fts_match(), "\"article\"*");
        assert_eq!(q.terms(), &["article"]);
    }

    #[test]
    fn test_multi_word_with_trailing_prefix() {
        let q = FtsQueryBuilder::build("limits on parliament amend").unwrap();
        assert_eq!(
            q.as_fts_match(),
            "\"limits\" \"on\" \"parliament\" \"amend\"*"
        );
        assert_eq!(
            q.as_relaxed_fts_match().as_deref(),
            Some(
                "(\"limits\" \"parliament\") OR (\"limits\" \"amend\") OR (\"parliament\" \"amend\")"
            )
        );
    }

    #[test]
    fn test_quoted_phrase() {
        let q = FtsQueryBuilder::build("\"Kesavananda Bharati\" constitution").unwrap();
        assert_eq!(
            q.as_fts_match(),
            "\"Kesavananda Bharati\" \"constitution\"*"
        );
        assert_eq!(q.terms(), &["Kesavananda Bharati", "constitution"]);
    }

    #[test]
    fn test_dangerous_characters_escaped() {
        let q = FtsQueryBuilder::build("SELECT * FROM (questions) WHERE 1=1; AND OR NOT").unwrap();
        assert!(!q.as_fts_match().contains('*'));
        assert!(!q.as_fts_match().contains('('));
        assert!(!q.as_fts_match().contains(')'));
        assert!(!q.as_fts_match().contains(';'));
    }

    #[test]
    fn test_article_legal_lookup() {
        let q = FtsQueryBuilder::build("Article 32").unwrap();
        assert_eq!(q.as_fts_match(), "\"Article\" \"32\"");
    }

    #[test]
    fn test_numeric_suffix_is_exact_instead_of_a_prefix() {
        let q = FtsQueryBuilder::build("Article 20").unwrap();
        assert_eq!(q.as_fts_match(), "\"Article\" \"20\"");

        let year = FtsQueryBuilder::build("calendar 2025").unwrap();
        assert_eq!(year.as_fts_match(), "\"calendar\" \"2025\"");
    }
}
