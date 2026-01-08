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

use memchr::memmem;
use regex::{Regex, RegexSet};
use serde_json::Value as JsonValue;
use std::collections::HashMap as StdHashMap;

/// A pre-built substring matcher using memchr's optimized memmem algorithm.
///
/// This struct stores both the needle string and a pre-built Finder for efficient
/// repeated substring searches. The Finder is created once during construction
/// and reused for all searches.
#[derive(Debug, Clone)]
pub struct ContainsMatcher {
    // Box to heap-allocate the needle so it doesn't move when ContainsMatcher is moved
    needle: Box<str>,
    // The Finder holds a pointer to the needle bytes
    // SAFETY: The needle is heap-allocated via Box, so moving ContainsMatcher
    // only moves the pointer, not the actual string data. The Finder's pointer
    // remains valid as long as we own the Box.
    finder: memmem::Finder<'static>,
}

impl ContainsMatcher {
    /// Create a new ContainsMatcher for the given needle string.
    pub fn new(needle: String) -> Self {
        let boxed_needle: Box<str> = needle.into_boxed_str();
        let needle_bytes: &[u8] = boxed_needle.as_bytes();

        // SAFETY: We're extending the lifetime to 'static. This is safe because:
        // 1. The needle is heap-allocated in a Box
        // 2. Moving this struct only moves the Box pointer, not the heap data
        // 3. The Finder's internal pointer to the needle bytes remains valid
        // 4. The needle lives as long as this struct
        let finder = unsafe {
            std::mem::transmute::<memmem::Finder<'_>, memmem::Finder<'static>>(memmem::Finder::new(
                needle_bytes,
            ))
        };

        Self {
            needle: boxed_needle,
            finder,
        }
    }

    /// Check if the haystack contains this needle using the pre-built finder.
    #[inline]
    pub fn is_contained_in(&self, haystack: &str) -> bool {
        self.finder.find(haystack.as_bytes()).is_some()
    }

    /// Get the needle pattern.
    #[inline]
    pub fn needle(&self) -> &str {
        &self.needle
    }
}

/// A string with optional ASCII case-insensitive matching.
///
/// For ASCII case-insensitive matching, compares bytes directly without allocation.
/// For non-ASCII case-insensitive matching, use a regex with (?i) flag instead.
#[derive(Debug, Clone)]
pub struct MaybeSensitiveStr {
    /// Pattern string
    s: String,
    /// True if matching should be case-sensitive
    ascii_case_sensitive: bool,
}

impl MaybeSensitiveStr {
    /// Create a new MaybeSensitiveStr.
    pub fn new(s: String, ascii_case_sensitive: bool) -> Self {
        Self {
            s,
            ascii_case_sensitive,
        }
    }

    /// Get the pattern string.
    #[inline]
    pub fn pattern(&self) -> &str {
        &self.s
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

    /// Check if a value starts with this pattern (ASCII case-insensitive).
    #[inline]
    pub fn starts_with(&self, value: &str) -> bool {
        if self.ascii_case_sensitive {
            value.starts_with(&self.s)
        } else {
            // ASCII case-insensitive comparison without allocation
            if value.len() < self.s.len() {
                return false;
            }
            value[..self.s.len()].eq_ignore_ascii_case(&self.s)
        }
    }

    /// Check if a value ends with this pattern (ASCII case-insensitive).
    #[inline]
    pub fn ends_with(&self, value: &str) -> bool {
        if self.ascii_case_sensitive {
            value.ends_with(&self.s)
        } else {
            // ASCII case-insensitive comparison without allocation
            if value.len() < self.s.len() {
                return false;
            }
            value[value.len() - self.s.len()..].eq_ignore_ascii_case(&self.s)
        }
    }
}

/// Optimized string predicate that can handle simple operations or multiple regexes.
///
/// This enum allows us to use fast string operations for simple cases and RegexSet
/// for regex patterns.
#[derive(Debug, Clone)]
pub enum StringPredicate {
    /// Simple string operations (starts_with, ends_with, contains).
    /// This is used when we only have simple string operations on a field.
    Simple {
        /// Optional starts_with check
        starts_with: Option<MaybeSensitiveStr>,
        /// Optional ends_with check
        ends_with: Option<MaybeSensitiveStr>,
        /// Pre-built substring matchers for case-sensitive contains checks
        contains: Vec<ContainsMatcher>,
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
    /// A predicate that never matches.
    /// Used when regex compilation fails or predicates are invalid.
    Never,
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

                // Check all contains using pre-built finders
                for matcher in contains {
                    if !matcher.is_contained_in(value) {
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
            StringPredicate::Never => false,
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
    /// Creates a pre-built ContainsMatcher for efficient substring searching.
    pub fn with_contains(mut self, needle: String) -> Self {
        if let StringPredicate::Simple { contains, .. } = &mut self {
            contains.push(ContainsMatcher::new(needle));
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

/// Object-based predicate matching for JSON bodies.
///
/// Mountebank supports matching against JSON objects, not just strings.
/// This enum handles the different types of object matching.
#[derive(Debug, Clone)]
pub enum ObjectPredicate {
    /// Subset match - the request object must contain all key-value pairs from the predicate
    /// (but can have additional fields).
    Equals(JsonValue),
    /// Exact match - the request object must exactly match the predicate object.
    DeepEquals(JsonValue),
    /// Contains - the request object must contain the predicate object as a subset.
    Contains(JsonValue),
    /// Regex match - each field in the predicate is a regex that must match the corresponding
    /// field in the request object.
    Matches(StdHashMap<String, Regex>),
}

impl ObjectPredicate {
    /// Check if this predicate matches the given JSON value.
    pub fn matches(&self, value: &JsonValue) -> bool {
        match self {
            ObjectPredicate::Equals(expected) => {
                // Subset match: all fields in expected must exist and match in value
                Self::is_subset(expected, value)
            }
            ObjectPredicate::DeepEquals(expected) => {
                // Exact match
                expected == value
            }
            ObjectPredicate::Contains(expected) => {
                // Subset match (same as Equals for objects)
                Self::is_subset(expected, value)
            }
            ObjectPredicate::Matches(regexes) => {
                // Each regex must match its corresponding field
                if let JsonValue::Object(obj) = value {
                    regexes.iter().all(|(key, regex)| {
                        obj.get(key)
                            .and_then(|v| v.as_str())
                            .map(|s| regex.is_match(s))
                            .unwrap_or(false)
                    })
                } else {
                    false
                }
            }
        }
    }

    /// Check if `subset` is a subset of `superset`.
    /// All fields in `subset` must exist and match in `superset`.
    fn is_subset(subset: &JsonValue, superset: &JsonValue) -> bool {
        match (subset, superset) {
            (JsonValue::Object(sub_obj), JsonValue::Object(super_obj)) => {
                // All keys in subset must exist in superset and have matching values
                sub_obj.iter().all(|(key, sub_value)| {
                    super_obj
                        .get(key)
                        .map(|super_value| Self::is_subset(sub_value, super_value))
                        .unwrap_or(false)
                })
            }
            (JsonValue::Array(sub_arr), JsonValue::Array(super_arr)) => {
                // For arrays, check if subset array is contained in superset array
                // This is a simple implementation; Mountebank's actual behavior may differ
                sub_arr.len() <= super_arr.len()
                    && sub_arr
                        .iter()
                        .all(|sub_item| super_arr.iter().any(|super_item| sub_item == super_item))
            }
            // For primitive values, they must be equal
            _ => subset == superset,
        }
    }
}

/// A predicate that can match either strings or JSON objects.
#[derive(Debug, Clone)]
pub enum ValuePredicate {
    /// String-based matching
    String(StringPredicate),
    /// Object-based matching (for JSON bodies)
    Object(ObjectPredicate),
}

impl ValuePredicate {
    /// Match against a string value.
    pub fn matches_str(&self, value: &str) -> bool {
        match self {
            ValuePredicate::String(pred) => pred.matches(value),
            ValuePredicate::Object(pred) => {
                // Try to parse as JSON
                if let Ok(json) = serde_json::from_str(value) {
                    pred.matches(&json)
                } else {
                    false
                }
            }
        }
    }

    /// Match against a JSON value.
    pub fn matches_json(&self, value: &JsonValue) -> bool {
        match self {
            ValuePredicate::String(pred) => {
                // Convert JSON to string and match
                if let Some(s) = value.as_str() {
                    pred.matches(s)
                } else {
                    // For non-string JSON, convert to string representation
                    pred.matches(&value.to_string())
                }
            }
            ValuePredicate::Object(pred) => pred.matches(value),
        }
    }
}

/// Field-level preprocessing and matching.
///
/// Wraps a ValuePredicate (string or object) with optional preprocessing like `except` patterns
/// and value extraction via jsonpath/xpath selectors.
#[derive(Debug, Clone)]
pub struct FieldPredicate {
    /// The matching predicate (string or object)
    pub predicate: ValuePredicate,
    /// Optional regex pattern to strip from values before matching (Mountebank `except` parameter)
    pub except: Option<Regex>,
    /// Optional selector for extracting values before matching (jsonpath/xpath)
    /// Only applicable to body field
    pub selector: Option<ValueSelector>,
}

impl FieldPredicate {
    /// Create a new FieldPredicate with a string predicate (no preprocessing).
    pub fn new(predicate: StringPredicate) -> Self {
        Self {
            predicate: ValuePredicate::String(predicate),
            except: None,
            selector: None,
        }
    }

    /// Create a new FieldPredicate with a value predicate (no preprocessing).
    pub fn new_value(predicate: ValuePredicate) -> Self {
        Self {
            predicate,
            except: None,
            selector: None,
        }
    }

    /// Create a new FieldPredicate with an object predicate.
    pub fn new_object(predicate: ObjectPredicate) -> Self {
        Self {
            predicate: ValuePredicate::Object(predicate),
            except: None,
            selector: None,
        }
    }

    /// Create a FieldPredicate with an except pattern.
    pub fn with_except(predicate: StringPredicate, except: Regex) -> Self {
        Self {
            predicate: ValuePredicate::String(predicate),
            except: Some(except),
            selector: None,
        }
    }

    /// Create a FieldPredicate with a value predicate and except pattern.
    pub fn with_except_value(predicate: ValuePredicate, except: Regex) -> Self {
        Self {
            predicate,
            except: Some(except),
            selector: None,
        }
    }

    /// Create a FieldPredicate with a selector.
    pub fn with_selector(predicate: StringPredicate, selector: ValueSelector) -> Self {
        Self {
            predicate: ValuePredicate::String(predicate),
            except: None,
            selector: Some(selector),
        }
    }

    /// Create a FieldPredicate with a value predicate and selector.
    pub fn with_selector_value(predicate: ValuePredicate, selector: ValueSelector) -> Self {
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
            predicate: ValuePredicate::String(predicate),
            except: Some(except),
            selector: Some(selector),
        }
    }

    /// Create a FieldPredicate with value predicate, except, and selector.
    pub fn with_except_and_selector_value(
        predicate: ValuePredicate,
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
                self.predicate.matches_str(&processed)
            }
            None => self.predicate.matches_str(value),
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

        // Note: contains is not supported for case-insensitive matching
        // Use regex with (?i) flag instead
    }

    #[test]
    fn test_string_predicate_simple() {
        let pred = StringPredicate::empty_simple()
            .with_starts_with(MaybeSensitiveStr::new("http://".to_string(), true))
            .with_contains(MaybeSensitiveStr::new("api".to_string(), true))
            .with_ends_with(MaybeSensitiveStr::new("json".to_string(), true));

        assert!(pred.matches("http://example.com/api/data.json"));
        assert!(!pred.matches("https://example.com/api/data.json")); // doesn't start with http://
        assert!(!pred.matches("http://example.com/users.json")); // doesn't contain api
        assert!(!pred.matches("http://example.com/api/data.xml")); // doesn't end with json
    }

    #[test]
    fn test_string_predicate_equals() {
        let pred = StringPredicate::empty_simple()
            .with_equals(MaybeSensitiveStr::new("GET".to_string(), true));

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
        pred.method
            .push(FieldPredicate::new(StringPredicate::empty_simple()));
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
