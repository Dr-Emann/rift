//! Optimized predicate matching using per-field organization.
//!
//! This module provides an optimized internal representation of predicates that organizes
//! matchers by field rather than by type. This improves cache locality and enables
//! optimizations like RegexSet for checking multiple patterns simultaneously.
//!
//! # Optimization Strategy
//!
//! Instead of storing predicates as a list of operations like:
//! ```yaml
//! - startsWith: { body: "abc" }
//! - contains: { body: "123" }
//! - contains: { body: "456" }
//! - matches: { path: '^/my_path/\d+$', body: 'busy-\d+' }
//! ```
//!
//! We reorganize to group by field:
//! ```yaml
//! body:
//!   startsWith: "abc"
//!   contains: ["123", "456"]
//!   matches: 'busy-\d+'
//! path:
//!   matches: '^/my_path/\d+$'
//! ```
//!
//! This allows us to:
//! 1. Process all predicates for a field in one pass over the field value
//! 2. Use RegexSet to check multiple regexes simultaneously
//! 3. Use optimized string search (memmem) for contains operations
//! 4. Improve cache locality

use regex::{Regex, RegexSet};
use std::borrow::Cow;

/// A string with optional case-insensitive matching.
///
/// For case-insensitive matching, stores both original and lowercase versions
/// to avoid repeated allocations during matching.
#[derive(Debug, Clone)]
pub struct MaybeSensitiveStr {
    /// Original string
    s: String,
    /// True if matching should be case-sensitive (ASCII)
    ascii_case_sensitive: bool,
    /// Cached lowercase version (only populated if case-insensitive)
    lower: Option<String>,
}

impl MaybeSensitiveStr {
    /// Create a new MaybeSensitiveStr.
    pub fn new(s: String, ascii_case_sensitive: bool) -> Self {
        let lower = if ascii_case_sensitive {
            None
        } else {
            Some(s.to_ascii_lowercase())
        };
        Self {
            s,
            ascii_case_sensitive,
            lower,
        }
    }

    /// Get the pattern to match against, handling case sensitivity.
    #[inline]
    pub fn pattern(&self) -> &str {
        if self.ascii_case_sensitive {
            &self.s
        } else {
            self.lower.as_ref().unwrap()
        }
    }

    /// Check if a value equals this pattern.
    #[inline]
    pub fn equals(&self, value: &str) -> bool {
        if self.ascii_case_sensitive {
            value == self.s
        } else {
            value.eq_ignore_ascii_case(&self.s)
        }
    }

    /// Check if a value starts with this pattern.
    #[inline]
    pub fn starts_with(&self, value: &str) -> bool {
        if self.ascii_case_sensitive {
            value.starts_with(&self.s)
        } else {
            value
                .to_ascii_lowercase()
                .starts_with(self.lower.as_ref().unwrap())
        }
    }

    /// Check if a value ends with this pattern.
    #[inline]
    pub fn ends_with(&self, value: &str) -> bool {
        if self.ascii_case_sensitive {
            value.ends_with(&self.s)
        } else {
            value
                .to_ascii_lowercase()
                .ends_with(self.lower.as_ref().unwrap())
        }
    }

    /// Check if a value contains this pattern.
    #[inline]
    pub fn contained_in(&self, value: &str) -> bool {
        if self.ascii_case_sensitive {
            value.contains(&self.s)
        } else {
            value
                .to_ascii_lowercase()
                .contains(self.lower.as_ref().unwrap())
        }
    }
}

/// Optimized string predicate that can handle simple operations or multiple regexes.
///
/// This enum allows us to use fast string operations for simple cases and RegexSet
/// for complex cases involving multiple regex patterns.
#[derive(Debug, Clone)]
pub enum StringPredicate {
    /// Simple string operations (starts_with, ends_with, contains).
    /// This is used when we only have simple string operations on a field.
    Simple {
        /// Optional starts_with check
        starts_with: Option<MaybeSensitiveStr>,
        /// Optional ends_with check
        ends_with: Option<MaybeSensitiveStr>,
        /// Optional contains checks (can have multiple)
        /// Using Vec for simplicity; could optimize to single value for common case
        contains: Vec<MaybeSensitiveStr>,
        /// Optional equals check
        equals: Option<MaybeSensitiveStr>,
    },
    /// Multiple regex patterns checked simultaneously using RegexSet.
    /// This is more efficient than checking multiple Regex instances separately.
    /// The RegexSet can report if all patterns matched using SetMatches::matched_all().
    Regexes {
        set: RegexSet,
        /// True if all regexes must match (AND), false if any can match (OR)
        require_all: bool,
    },
    /// A combination of simple operations AND regexes.
    /// All simple operations must match, and the regex set must match according to require_all.
    Combined {
        simple: Box<StringPredicate>,
        regexes: RegexSet,
        require_all_regexes: bool,
    },
}

impl StringPredicate {
    /// Create a simple predicate with no constraints.
    pub fn empty_simple() -> Self {
        StringPredicate::Simple {
            starts_with: None,
            ends_with: None,
            contains: Vec::new(),
            equals: None,
        }
    }

    /// Check if this predicate matches the given value.
    ///
    /// # Arguments
    /// * `value` - The string value to match against
    ///
    /// # Returns
    /// `true` if all constraints in this predicate match the value
    pub fn matches(&self, value: &str) -> bool {
        match self {
            StringPredicate::Simple {
                starts_with,
                ends_with,
                contains,
                equals,
            } => {
                // Check equals first (most restrictive)
                if let Some(eq) = equals {
                    if !eq.equals(value) {
                        return false;
                    }
                }

                // Check starts_with
                if let Some(sw) = starts_with {
                    if !sw.starts_with(value) {
                        return false;
                    }
                }

                // Check ends_with
                if let Some(ew) = ends_with {
                    if !ew.ends_with(value) {
                        return false;
                    }
                }

                // Check all contains
                for c in contains {
                    if !c.contained_in(value) {
                        return false;
                    }
                }

                true
            }
            StringPredicate::Regexes { set, require_all } => {
                let matches = set.matches(value);
                if *require_all {
                    matches.matched_all()
                } else {
                    matches.matched_any()
                }
            }
            StringPredicate::Combined {
                simple,
                regexes,
                require_all_regexes,
            } => {
                // Both simple and regexes must match
                if !simple.matches(value) {
                    return false;
                }

                let matches = regexes.matches(value);
                if *require_all_regexes {
                    matches.matched_all()
                } else {
                    matches.matched_any()
                }
            }
        }
    }

    /// Add a starts_with constraint to a Simple predicate.
    pub fn with_starts_with(mut self, pattern: MaybeSensitiveStr) -> Self {
        if let StringPredicate::Simple { starts_with, .. } = &mut self {
            *starts_with = Some(pattern);
        }
        self
    }

    /// Add an ends_with constraint to a Simple predicate.
    pub fn with_ends_with(mut self, pattern: MaybeSensitiveStr) -> Self {
        if let StringPredicate::Simple { ends_with, .. } = &mut self {
            *ends_with = Some(pattern);
        }
        self
    }

    /// Add a contains constraint to a Simple predicate.
    pub fn with_contains(mut self, pattern: MaybeSensitiveStr) -> Self {
        if let StringPredicate::Simple { contains, .. } = &mut self {
            contains.push(pattern);
        }
        self
    }

    /// Add an equals constraint to a Simple predicate.
    pub fn with_equals(mut self, pattern: MaybeSensitiveStr) -> Self {
        if let StringPredicate::Simple { equals, .. } = &mut self {
            *equals = Some(pattern);
        }
        self
    }
}

/// Field-level preprocessing and matching.
///
/// Wraps a StringPredicate with optional preprocessing like `except` patterns
/// and value extraction via jsonpath/xpath selectors.
#[derive(Debug, Clone)]
pub struct FieldPredicate {
    /// The string matching predicate
    pub predicate: StringPredicate,
    /// Optional regex pattern to strip from values before matching (Mountebank `except` parameter)
    pub except: Option<Regex>,
    /// Optional selector for extracting values before matching (jsonpath/xpath)
    /// Only applicable to body field
    pub selector: Option<ValueSelector>,
}

impl FieldPredicate {
    /// Create a new FieldPredicate with just the predicate (no preprocessing).
    pub fn new(predicate: StringPredicate) -> Self {
        Self {
            predicate,
            except: None,
            selector: None,
        }
    }

    /// Create a FieldPredicate with an except pattern.
    pub fn with_except(predicate: StringPredicate, except: Regex) -> Self {
        Self {
            predicate,
            except: Some(except),
            selector: None,
        }
    }

    /// Create a FieldPredicate with a selector.
    pub fn with_selector(predicate: StringPredicate, selector: ValueSelector) -> Self {
        Self {
            predicate,
            except: None,
            selector: Some(selector),
        }
    }

    /// Create a FieldPredicate with both except and selector.
    pub fn with_except_and_selector(
        predicate: StringPredicate,
        except: Regex,
        selector: ValueSelector,
    ) -> Self {
        Self {
            predicate,
            except: Some(except),
            selector: Some(selector),
        }
    }

    /// Match a value, applying except pattern if present.
    ///
    /// Note: Selector extraction should be done before calling this method.
    #[inline]
    pub fn matches(&self, value: &str) -> bool {
        match &self.except {
            Some(except) => {
                // Strip the except pattern and match against the result
                let processed = except.replace_all(value, "");
                self.predicate.matches(&processed)
            }
            None => self.predicate.matches(value),
        }
    }
}

/// Selector for extracting values before matching (jsonpath or xpath).
#[derive(Debug, Clone)]
pub enum ValueSelector {
    /// JsonPath selector
    JsonPath(String),
    /// XPath selector (with optional namespaces)
    XPath {
        selector: String,
        namespaces: Option<std::collections::HashMap<String, String>>,
    },
}

/// Optimized predicates organized by field.
///
/// This structure groups all predicates by the field they operate on (body, path, etc.),
/// allowing for better cache locality and optimization opportunities.
///
/// # Selector Handling
///
/// For predicates with selectors (jsonpath/xpath), each unique selector requires
/// a separate extraction and match. Predicates with the same selector can be optimized
/// together. Therefore, all fields that can have selectors are stored as Vec to support
/// multiple predicates with different selectors.
#[derive(Debug, Clone)]
pub struct OptimizedPredicates {
    /// Predicates for the HTTP method field
    /// Vec supports multiple predicates with different selectors
    pub method: Vec<FieldPredicate>,
    /// Predicates for the path field
    /// Vec supports multiple predicates with different selectors
    pub path: Vec<FieldPredicate>,
    /// Predicates for the body field
    /// Vec supports multiple predicates with different selectors (jsonpath/xpath)
    /// Predicates with the same selector are grouped together during optimization
    pub body: Vec<FieldPredicate>,
    /// Predicates for specific query parameters
    /// Key is the query parameter name, Vec for each supports different selectors
    pub query: Vec<(String, Vec<FieldPredicate>)>,
    /// Predicates for specific headers
    /// Key is the header name (lowercase), Vec for each supports different selectors
    pub headers: Vec<(String, Vec<FieldPredicate>)>,
    /// Predicates for requestFrom field
    /// Vec supports multiple predicates with different selectors
    pub request_from: Vec<FieldPredicate>,
    /// Predicates for ip field
    /// Vec supports multiple predicates with different selectors
    pub ip: Vec<FieldPredicate>,
    /// Predicates for form fields
    /// Key is the form field name, Vec for each supports different selectors
    pub form: Vec<(String, Vec<FieldPredicate>)>,
}

impl OptimizedPredicates {
    /// Create an empty set of optimized predicates.
    pub fn new() -> Self {
        Self {
            method: Vec::new(),
            path: Vec::new(),
            body: Vec::new(),
            query: Vec::new(),
            headers: Vec::new(),
            request_from: Vec::new(),
            ip: Vec::new(),
            form: Vec::new(),
        }
    }

    /// Check if a request matches these predicates.
    ///
    /// # Arguments
    /// * `method` - HTTP method (e.g., "GET", "POST")
    /// * `path` - Request path
    /// * `query` - Query parameters as a HashMap
    /// * `headers` - Request headers as a HashMap (keys should be lowercase)
    /// * `body` - Request body as a string
    /// * `request_from` - IP:port of the requester
    /// * `client_ip` - IP address of the client
    /// * `form` - Form data as a HashMap
    ///
    /// # Returns
    /// `true` if all predicates match the request
    #[allow(clippy::too_many_arguments)]
    pub fn matches(
        &self,
        method: &str,
        path: &str,
        query: &std::collections::HashMap<String, String>,
        headers: &std::collections::HashMap<String, String>,
        body: Option<&str>,
        request_from: Option<&str>,
        client_ip: Option<&str>,
        form: Option<&std::collections::HashMap<String, String>>,
    ) -> bool {
        // Helper to match a value with selector extraction
        let match_with_selector = |pred: &FieldPredicate, value: &str| -> bool {
            let value_to_match = match &pred.selector {
                Some(ValueSelector::JsonPath(_selector)) => {
                    // Extract using jsonpath
                    // TODO: Implement jsonpath extraction
                    // For now, use the full value
                    value
                }
                Some(ValueSelector::XPath { .. }) => {
                    // Extract using xpath
                    // TODO: Implement xpath extraction
                    // For now, use the full value
                    value
                }
                None => value,
            };
            pred.matches(value_to_match)
        };

        // Check method predicates
        for pred in &self.method {
            if !match_with_selector(pred, method) {
                return false;
            }
        }

        // Check path predicates
        for pred in &self.path {
            if !match_with_selector(pred, path) {
                return false;
            }
        }

        // Check body predicates
        let body_str = body.unwrap_or("");
        for pred in &self.body {
            if !match_with_selector(pred, body_str) {
                return false;
            }
        }

        // Check request_from predicates
        let rf = request_from.unwrap_or("");
        for pred in &self.request_from {
            if !match_with_selector(pred, rf) {
                return false;
            }
        }

        // Check ip predicates
        let ip_str = client_ip.unwrap_or("");
        for pred in &self.ip {
            if !match_with_selector(pred, ip_str) {
                return false;
            }
        }

        // Check query parameters
        for (param_name, preds) in &self.query {
            match query.get(param_name) {
                Some(value) => {
                    // All predicates for this query parameter must match
                    for pred in preds {
                        if !match_with_selector(pred, value) {
                            return false;
                        }
                    }
                }
                None => return false, // Required query parameter not present
            }
        }

        // Check headers
        for (header_name, preds) in &self.headers {
            match headers.get(header_name) {
                Some(value) => {
                    // All predicates for this header must match
                    for pred in preds {
                        if !match_with_selector(pred, value) {
                            return false;
                        }
                    }
                }
                None => return false, // Required header not present
            }
        }

        // Check form fields
        for (field_name, preds) in &self.form {
            match form.and_then(|f| f.get(field_name)) {
                Some(value) => {
                    // All predicates for this form field must match
                    for pred in preds {
                        if !match_with_selector(pred, value) {
                            return false;
                        }
                    }
                }
                None => return false, // Required form field not present
            }
        }

        true
    }

    /// Check if these predicates have any constraints.
    pub fn is_empty(&self) -> bool {
        self.method.is_empty()
            && self.path.is_empty()
            && self.body.is_empty()
            && self.query.is_empty()
            && self.headers.is_empty()
            && self.request_from.is_empty()
            && self.ip.is_empty()
            && self.form.is_empty()
    }
}

impl Default for OptimizedPredicates {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maybe_sensitive_str_case_sensitive() {
        let s = MaybeSensitiveStr::new("Test".to_string(), true);

        assert!(s.equals("Test"));
        assert!(!s.equals("test"));
        assert!(!s.equals("TEST"));

        assert!(s.starts_with("Testing"));
        assert!(!s.starts_with("testing"));

        assert!(s.ends_with("aTest"));
        assert!(!s.ends_with("atest"));

        assert!(s.contained_in("This is a Test string"));
        assert!(!s.contained_in("This is a test string"));
    }

    #[test]
    fn test_maybe_sensitive_str_case_insensitive() {
        let s = MaybeSensitiveStr::new("Test".to_string(), false);

        assert!(s.equals("Test"));
        assert!(s.equals("test"));
        assert!(s.equals("TEST"));
        assert!(s.equals("TeSt"));

        assert!(s.starts_with("Testing"));
        assert!(s.starts_with("testing"));
        assert!(s.starts_with("TESTING"));

        assert!(s.ends_with("aTest"));
        assert!(s.ends_with("atest"));
        assert!(s.ends_with("ATEST"));

        assert!(s.contained_in("This is a Test string"));
        assert!(s.contained_in("This is a test string"));
        assert!(s.contained_in("This is a TEST string"));
    }

    #[test]
    fn test_string_predicate_simple() {
        let pred = StringPredicate::empty_simple()
            .with_starts_with(MaybeSensitiveStr::new("http".to_string(), true))
            .with_contains(MaybeSensitiveStr::new("api".to_string(), true))
            .with_ends_with(MaybeSensitiveStr::new("json".to_string(), true));

        assert!(pred.matches("http://example.com/api/data.json"));
        assert!(!pred.matches("https://example.com/api/data.json")); // doesn't start with http
        assert!(!pred.matches("http://example.com/users.json")); // doesn't contain api
        assert!(!pred.matches("http://example.com/api/data.xml")); // doesn't end with json
    }

    #[test]
    fn test_string_predicate_equals() {
        let pred =
            StringPredicate::empty_simple().with_equals(MaybeSensitiveStr::new("GET".to_string(), true));

        assert!(pred.matches("GET"));
        assert!(!pred.matches("POST"));
        assert!(!pred.matches("get")); // case sensitive
    }

    #[test]
    fn test_string_predicate_regexes_all() {
        let patterns = vec![r"^/api/", r"/users/", r"/\d+$"];
        let set = RegexSet::new(patterns).unwrap();

        let pred = StringPredicate::Regexes {
            set,
            require_all: true,
        };

        assert!(pred.matches("/api/users/123")); // matches all patterns
        assert!(!pred.matches("/api/posts/123")); // doesn't match /users/
        assert!(!pred.matches("/api/users/abc")); // doesn't match /\d+$
    }

    #[test]
    fn test_string_predicate_regexes_any() {
        let patterns = vec![r"^GET$", r"^POST$", r"^PUT$"];
        let set = RegexSet::new(patterns).unwrap();

        let pred = StringPredicate::Regexes {
            set,
            require_all: false,
        };

        assert!(pred.matches("GET"));
        assert!(pred.matches("POST"));
        assert!(pred.matches("PUT"));
        assert!(!pred.matches("DELETE"));
    }

    #[test]
    fn test_string_predicate_combined() {
        let simple = Box::new(
            StringPredicate::empty_simple()
                .with_starts_with(MaybeSensitiveStr::new("/api".to_string(), true)),
        );

        let patterns = vec![r"/users/", r"/\d+"];
        let regexes = RegexSet::new(patterns).unwrap();

        let pred = StringPredicate::Combined {
            simple,
            regexes,
            require_all_regexes: true,
        };

        assert!(pred.matches("/api/users/123")); // starts with /api AND matches both regexes
        assert!(!pred.matches("/api/posts/123")); // doesn't match /users/ regex
        assert!(!pred.matches("/v1/users/123")); // doesn't start with /api
    }

    #[test]
    fn test_optimized_predicates_empty() {
        let pred = OptimizedPredicates::new();
        assert!(pred.is_empty());
    }

    #[test]
    fn test_optimized_predicates_not_empty() {
        let mut pred = OptimizedPredicates::new();
        pred.method.push(FieldPredicate::new(StringPredicate::empty_simple()));
        assert!(!pred.is_empty());
    }

    #[test]
    fn test_field_predicate_with_except() {
        let pred = StringPredicate::empty_simple()
            .with_equals(MaybeSensitiveStr::new("Hello World".to_string(), true));

        // Without except - doesn't match
        let field_pred = FieldPredicate::new(pred.clone());
        assert!(!field_pred.matches("Hello123 World456"));

        // With except - strips digits and matches
        let except_regex = Regex::new(r"\d+").unwrap();
        let field_pred_with_except = FieldPredicate::with_except(pred, except_regex);
        assert!(field_pred_with_except.matches("Hello123 World456"));
    }
}
