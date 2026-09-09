//! Safe, literal FTS5 query compilation. User text never becomes FTS syntax.

#[derive(Debug, Clone)]
pub struct FtsQueryOptions {
    pub enable_prefix_matching: bool,
    /// Standalone prefixes need two letters; an anchored final word may use one.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledFtsQuery {
    fts_expr: String,
    terms: Vec<String>,
    phrases: Vec<bool>,
    prefixes: Vec<bool>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchTerm {
    pub text: String,
    pub prefix: bool,
}

impl CompiledFtsQuery {
    pub fn as_fts_match(&self) -> &str {
        &self.fts_expr
    }
    pub fn terms(&self) -> &[String] {
        &self.terms
    }
    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }
    pub fn is_phrase(&self, index: usize) -> bool {
        self.phrases[index]
    }
    pub fn has_trailing_prefix(&self) -> bool {
        self.prefixes.last() == Some(&true)
    }
    pub fn is_prefix(&self, index: usize) -> bool {
        self.prefixes[index]
    }
    pub fn with_prefixes(&self, prefixes: Vec<bool>) -> Self {
        assert_eq!(prefixes.len(), self.terms.len());
        let mut query = self.clone();
        query.prefixes = prefixes;
        query.with_terms(self.terms.clone())
    }
    pub fn word_patterns(&self) -> Vec<SearchTerm> {
        self.terms
            .iter()
            .enumerate()
            .flat_map(|(index, term)| {
                term.split_whitespace().map(move |word| SearchTerm {
                    text: word.to_string(),
                    prefix: self.prefixes[index],
                })
            })
            .collect()
    }
    /// User-readable spelling, retaining closed phrases without exposing FTS syntax.
    pub fn display_text(&self) -> String {
        self.terms
            .iter()
            .enumerate()
            .map(|(index, term)| {
                if self.phrases[index] {
                    quote_term(term, false)
                } else {
                    term.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn exact_spelling_text(&self) -> String {
        self.terms
            .iter()
            .map(|term| quote_term(term, false))
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn term_match(&self, index: usize) -> String {
        quote_term(&self.terms[index], self.prefixes[index])
    }

    pub fn required_phrases_match(&self) -> Option<String> {
        let phrases = self
            .terms
            .iter()
            .enumerate()
            .filter(|(index, _)| self.phrases[*index])
            .map(|(_, term)| quote_term(term, false))
            .collect::<Vec<_>>();
        (!phrases.is_empty()).then(|| phrases.join(" AND "))
    }

    pub fn entirely_quoted(&self) -> bool {
        !self.phrases.is_empty() && self.phrases.iter().all(|phrase| *phrase)
    }

    /// Preserve phrase boundaries and prefix intent when correcting a word.
    pub fn with_terms(&self, terms: Vec<String>) -> Self {
        assert_eq!(terms.len(), self.terms.len());
        let mut corrected = self.clone();
        corrected.terms = terms;
        corrected.fts_expr = (0..corrected.terms.len())
            .map(|i| corrected.term_match(i))
            .collect::<Vec<_>>()
            .join(" ");
        corrected
    }

    /// Recover pairs of meaningful terms, retaining their original semantics.
    pub fn as_relaxed_fts_match(&self) -> Option<String> {
        if let Some(required) = self.required_phrases_match() {
            let optional = self
                .terms
                .iter()
                .enumerate()
                .filter(|(index, term)| {
                    !self.phrases[*index] && (self.prefixes[*index] || !is_relaxed_stop_word(term))
                })
                .map(|(index, _)| self.term_match(index))
                .collect::<Vec<_>>();
            return (optional.len() >= 2)
                .then(|| format!("{required} AND ({})", optional.join(" OR ")));
        }
        let selected = self
            .terms
            .iter()
            .enumerate()
            .filter(|(i, term)| {
                self.phrases[*i] || self.prefixes[*i] || !is_relaxed_stop_word(term)
            })
            .map(|(i, _)| self.term_match(i))
            .collect::<Vec<_>>();
        if selected.len() < 2 {
            return None;
        }
        if self.terms.len() == 2 {
            // Both words are already mandatory in the primary query.
            return None;
        }
        // Factor each word's possible partners into one group. Enumerating
        // every pair creates thousands of top-level OR branches for a pasted
        // question; these groups express the same at-least-two rule.
        Some(
            (0..selected.len() - 1)
                .map(|left| {
                    format!(
                        "({} AND ({}))",
                        selected[left],
                        selected[left + 1..].join(" OR ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" OR "),
        )
    }
}

fn quote_term(term: &str, prefix: bool) -> String {
    format!(
        "\"{}\"{}",
        term.replace('"', "\"\""),
        if prefix { "*" } else { "" }
    )
}

pub(crate) fn is_relaxed_stop_word(term: &str) -> bool {
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

/// Punctuation separates words just as it does in SQLite's unicode61 index.
/// Keeping boundaries avoids turning `India’s`, `climate/change`, or a pasted
/// em dash into unrelated concatenated words.
pub fn normalized_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase)
        .collect()
}

pub struct FtsQueryBuilder;

impl FtsQueryBuilder {
    pub fn build(input: &str) -> Option<CompiledFtsQuery> {
        Self::build_with_options(input, &FtsQueryOptions::default())
    }

    pub fn build_with_options(input: &str, options: &FtsQueryOptions) -> Option<CompiledFtsQuery> {
        let mut terms = Vec::new();
        let mut phrases = Vec::new();
        let mut in_quotes = false;
        let mut buffer = String::new();
        let push =
            |buffer: &str, phrase: bool, terms: &mut Vec<String>, phrases: &mut Vec<bool>| {
                let words = normalized_words(buffer);
                if phrase && !words.is_empty() {
                    terms.push(words.join(" "));
                    phrases.push(true);
                } else {
                    for word in words {
                        terms.push(word);
                        phrases.push(false);
                    }
                }
            };
        for ch in input.chars() {
            if matches!(ch, '"' | '“' | '”') {
                push(&buffer, in_quotes, &mut terms, &mut phrases);
                buffer.clear();
                in_quotes = !in_quotes;
            } else {
                buffer.push(ch);
            }
        }
        // An unfinished quote is still being typed. Treat its words normally
        // until the closing quote establishes an exact phrase.
        push(&buffer, false, &mut terms, &mut phrases);
        if terms.is_empty() {
            return None;
        }
        let mut prefixes = vec![false; terms.len()];
        let last = terms.len() - 1;
        let min_length = if last > 0 { 1 } else { options.min_prefix_len };
        prefixes[last] = options.enable_prefix_matching
            && !phrases[last]
            && terms[last].chars().count() >= min_length
            && terms[last].chars().all(char::is_alphabetic);
        // Quoted FTS keywords are ordinary words and can safely be prefixes:
        // `world or` must continue to find `world organization`.
        let query = CompiledFtsQuery {
            fts_expr: String::new(),
            terms,
            phrases,
            prefixes,
        };
        Some(query.with_terms(query.terms.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compilation_handles_typing_punctuation_and_quotes() {
        for (input, expression) in [
            ("silver n", "\"silver\" \"n\"*"),
            ("silver no", "\"silver\" \"no\"*"),
            ("world OR", "\"world\" \"or\"*"),
            ("silver/notice", "\"silver\" \"notice\"*"),
            ("silver—notice", "\"silver\" \"notice\"*"),
            ("India’s climate", "\"india\" \"s\" \"climate\"*"),
            ("silver notice ***", "\"silver\" \"notice\"*"),
            ("\"silver notice\"", "\"silver notice\""),
            ("“silver notice”", "\"silver notice\""),
            ("\"silver n", "\"silver\" \"n\"*"),
            ("Article 20", "\"article\" \"20\""),
            ("calendar 2025", "\"calendar\" \"2025\""),
            ("SDG 5", "\"sdg\" \"5\""),
            ("n", "\"n\""),
        ] {
            assert_eq!(
                FtsQueryBuilder::build(input).unwrap().as_fts_match(),
                expression,
                "{input}"
            );
        }
        for input in ["", "   ", "***", "\"\""] {
            assert_eq!(FtsQueryBuilder::build(input), None);
        }
    }

    #[test]
    fn correction_and_relaxation_preserve_phrases_and_prefixes() {
        let query = FtsQueryBuilder::build("\"silver notice\" crimnal a").unwrap();
        let corrected =
            query.with_terms(vec!["silver notice".into(), "criminal".into(), "a".into()]);
        assert_eq!(
            corrected.as_fts_match(),
            "\"silver notice\" \"criminal\" \"a\"*"
        );
        assert_eq!(
            corrected.as_relaxed_fts_match().unwrap(),
            "\"silver notice\" AND (\"criminal\" OR \"a\"*)"
        );
    }
}
