//! Mountebank-compatible predicate system for request matching.
//!
//! This module provides a comprehensive predicate system that supports all Mountebank
//! predicate operators (equals, contains, startsWith, endsWith, matches, exists, deepEquals)
//! with logical operators (AND, OR, NOT) and predicate parameters (caseSensitive, except).
//!
//! # Design Goals
//!
//! 1. **Mountebank Compatibility**: Support all Mountebank predicate types
//! 2. **Performance**: Pre-compile regexes, efficient string matching
//! 3. **Runtime Updates**: Designed for hot-reload with imposter support
//! 4. **Backward Compatibility**: Existing Rift configs continue to work

// Allow dead code while predicate system is being fully integrated
#![allow(dead_code)]

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// String matching operator for comparing string values.
///
/// Supports all Mountebank string matching operations.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum StringMatcher {
    /// Exact string equality
    #[serde(rename = "equals")]
    Equals(String),

    /// String contains substring
    #[serde(rename = "contains")]
    Contains(String),

    /// String starts with prefix
    #[serde(rename = "startsWith")]
    StartsWith(String),

    /// String ends with suffix
    #[serde(rename = "endsWith")]
    EndsWith(String),

    /// Regex pattern match
    #[serde(rename = "matches")]
    Matches(String),

    /// Field existence check (value is whether field should exist)
    #[serde(rename = "exists")]
    Exists(bool),
}

impl Default for StringMatcher {
    fn default() -> Self {
        StringMatcher::Exists(true)
    }
}

/// Options that modify predicate matching behavior.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PredicateOptions {
    /// Whether matching is case-sensitive (default: true for Rift, false for Mountebank)
    #[serde(default = "default_case_sensitive")]
    pub case_sensitive: bool,

    /// Regex pattern to strip from value before matching
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub except: Option<String>,

    /// Negate the match result (NOT operator)
    #[serde(default, skip_serializing_if = "is_false")]
    pub not: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for PredicateOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true, // Rift default - more performant
            except: None,
            not: false,
        }
    }
}

fn default_case_sensitive() -> bool {
    true // Rift default - more performant
}

/// Compiled string matcher for efficient runtime evaluation.
#[derive(Debug, Clone)]
pub enum CompiledStringMatcher {
    Equals { value: String, lower: String },
    Contains { value: String, lower: String },
    StartsWith { value: String, lower: String },
    EndsWith { value: String, lower: String },
    Matches(Arc<Regex>),
    Exists(bool),
}

/// Compiled except regex for stripping patterns before matching.
#[derive(Debug, Clone)]
pub struct CompiledExcept {
    pub regex: Arc<Regex>,
}

impl CompiledExcept {
    /// Compile an except regex pattern.
    pub fn compile(pattern: &str) -> Result<Self, regex::Error> {
        Ok(CompiledExcept {
            regex: Arc::new(Regex::new(pattern)?),
        })
    }

    /// Apply the except pattern, stripping matching content from the value.
    pub fn apply(&self, value: &str) -> String {
        self.regex.replace_all(value, "").to_string()
    }
}

impl CompiledStringMatcher {
    /// Compile a StringMatcher into an efficient runtime form.
    pub fn compile(matcher: &StringMatcher) -> Result<Self, regex::Error> {
        match matcher {
            StringMatcher::Equals(v) => Ok(CompiledStringMatcher::Equals {
                value: v.clone(),
                lower: v.to_lowercase(),
            }),
            StringMatcher::Contains(v) => Ok(CompiledStringMatcher::Contains {
                value: v.clone(),
                lower: v.to_lowercase(),
            }),
            StringMatcher::StartsWith(v) => Ok(CompiledStringMatcher::StartsWith {
                value: v.clone(),
                lower: v.to_lowercase(),
            }),
            StringMatcher::EndsWith(v) => Ok(CompiledStringMatcher::EndsWith {
                value: v.clone(),
                lower: v.to_lowercase(),
            }),
            StringMatcher::Matches(pattern) => {
                let regex = Regex::new(pattern)?;
                Ok(CompiledStringMatcher::Matches(Arc::new(regex)))
            }
            StringMatcher::Exists(exists) => Ok(CompiledStringMatcher::Exists(*exists)),
        }
    }

    /// Check if a value matches this matcher.
    ///
    /// # Arguments
    /// * `value` - The value to match against (None if field doesn't exist)
    /// * `case_sensitive` - Whether to perform case-sensitive matching
    pub fn matches(&self, value: Option<&str>, case_sensitive: bool) -> bool {
        match (self, value) {
            // Exists check
            (CompiledStringMatcher::Exists(should_exist), v) => {
                let does_exist = v.is_some();
                *should_exist == does_exist
            }

            // For all other matchers, value must exist
            (_, None) => false,

            (
                CompiledStringMatcher::Equals {
                    value: pattern,
                    lower,
                },
                Some(v),
            ) => {
                if case_sensitive {
                    v == pattern
                } else {
                    v.to_lowercase() == *lower
                }
            }

            (
                CompiledStringMatcher::Contains {
                    value: pattern,
                    lower,
                },
                Some(v),
            ) => {
                if case_sensitive {
                    v.contains(pattern.as_str())
                } else {
                    v.to_lowercase().contains(lower.as_str())
                }
            }

            (
                CompiledStringMatcher::StartsWith {
                    value: pattern,
                    lower,
                },
                Some(v),
            ) => {
                if case_sensitive {
                    v.starts_with(pattern.as_str())
                } else {
                    v.to_lowercase().starts_with(lower.as_str())
                }
            }

            (
                CompiledStringMatcher::EndsWith {
                    value: pattern,
                    lower,
                },
                Some(v),
            ) => {
                if case_sensitive {
                    v.ends_with(pattern.as_str())
                } else {
                    v.to_lowercase().ends_with(lower.as_str())
                }
            }

            (CompiledStringMatcher::Matches(regex), Some(v)) => {
                // Regex matching - case sensitivity should be in the pattern itself
                regex.is_match(v)
            }
        }
    }

    /// Check if a value matches this matcher, applying an optional except pattern first.
    ///
    /// The except pattern strips matching content from the value before comparison.
    pub fn matches_with_except(
        &self,
        value: Option<&str>,
        case_sensitive: bool,
        except: Option<&CompiledExcept>,
    ) -> bool {
        // Apply except pattern if present
        let processed_value = match (value, except) {
            (Some(v), Some(exc)) => Some(exc.apply(v)),
            (Some(v), None) => Some(v.to_string()),
            (None, _) => None,
        };

        self.matches(processed_value.as_deref(), case_sensitive)
    }
}

/// Header matching configuration with full predicate support.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum HeaderMatcher {
    /// Simple exact match (backward compatible): { name: "X-Api-Key", value: "secret" }
    Simple { name: String, value: String },

    /// OR predicate - matches if ANY of the matchers match
    Or {
        name: String,
        or: Vec<StringMatcher>,
        #[serde(flatten, default)]
        options: PredicateOptions,
    },

    /// Full predicate match with operators
    Full {
        name: String,
        #[serde(flatten)]
        matcher: StringMatcher,
        #[serde(flatten, default)]
        options: PredicateOptions,
    },
}

impl HeaderMatcher {
    /// Get the header name for this matcher.
    pub fn name(&self) -> &str {
        match self {
            HeaderMatcher::Simple { name, .. } => name,
            HeaderMatcher::Full { name, .. } => name,
            HeaderMatcher::Or { name, .. } => name,
        }
    }
}

/// Compiled single or OR header matcher.
#[derive(Debug, Clone)]
pub enum CompiledHeaderMatcherInner {
    Single(CompiledStringMatcher),
    Or(Vec<CompiledStringMatcher>),
}

/// Compiled header matcher for efficient runtime evaluation.
#[derive(Debug, Clone)]
pub struct CompiledHeaderMatcher {
    /// Header name (lowercased for HTTP header matching)
    pub name: String,
    /// Compiled matcher(s)
    pub matcher: CompiledHeaderMatcherInner,
    /// Predicate options
    pub case_sensitive: bool,
    /// Negate the match result (NOT operator)
    pub not: bool,
    /// Optional except pattern for stripping content before matching
    pub except: Option<CompiledExcept>,
}

impl CompiledHeaderMatcher {
    /// Compile a HeaderMatcher configuration.
    pub fn compile(config: &HeaderMatcher) -> Result<Self, regex::Error> {
        match config {
            HeaderMatcher::Simple { name, value } => Ok(CompiledHeaderMatcher {
                name: name.to_lowercase(),
                matcher: CompiledHeaderMatcherInner::Single(CompiledStringMatcher::Equals {
                    value: value.clone(),
                    lower: value.to_lowercase(),
                }),
                case_sensitive: true, // Default for backward compatibility
                not: false,
                except: None,
            }),
            HeaderMatcher::Or { name, or, options } => {
                let compiled: Result<Vec<_>, _> =
                    or.iter().map(CompiledStringMatcher::compile).collect();
                let except = options
                    .except
                    .as_ref()
                    .map(|p| CompiledExcept::compile(p))
                    .transpose()?;
                Ok(CompiledHeaderMatcher {
                    name: name.to_lowercase(),
                    matcher: CompiledHeaderMatcherInner::Or(compiled?),
                    case_sensitive: options.case_sensitive,
                    not: options.not,
                    except,
                })
            }
            HeaderMatcher::Full {
                name,
                matcher,
                options,
            } => {
                let except = options
                    .except
                    .as_ref()
                    .map(|p| CompiledExcept::compile(p))
                    .transpose()?;
                Ok(CompiledHeaderMatcher {
                    name: name.to_lowercase(),
                    matcher: CompiledHeaderMatcherInner::Single(CompiledStringMatcher::compile(
                        matcher,
                    )?),
                    case_sensitive: options.case_sensitive,
                    not: options.not,
                    except,
                })
            }
        }
    }

    /// Check if a header value matches.
    pub fn matches(&self, value: Option<&str>) -> bool {
        let result = match &self.matcher {
            CompiledHeaderMatcherInner::Single(m) => {
                m.matches_with_except(value, self.case_sensitive, self.except.as_ref())
            }
            CompiledHeaderMatcherInner::Or(matchers) => matchers
                .iter()
                .any(|m| m.matches_with_except(value, self.case_sensitive, self.except.as_ref())),
        };
        if self.not {
            !result
        } else {
            result
        }
    }
}

/// Query parameter matching configuration.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum QueryMatcher {
    /// Simple exact match: { name: "page", value: "1" }
    Simple { name: String, value: String },

    /// OR predicate - matches if ANY of the matchers match
    Or {
        name: String,
        or: Vec<StringMatcher>,
        #[serde(flatten, default)]
        options: PredicateOptions,
    },

    /// Full predicate match with operators
    Full {
        name: String,
        #[serde(flatten)]
        matcher: StringMatcher,
        #[serde(flatten, default)]
        options: PredicateOptions,
    },
}

impl QueryMatcher {
    /// Get the query parameter name.
    pub fn name(&self) -> &str {
        match self {
            QueryMatcher::Simple { name, .. } => name,
            QueryMatcher::Full { name, .. } => name,
            QueryMatcher::Or { name, .. } => name,
        }
    }
}

/// Compiled single or OR query matcher.
#[derive(Debug, Clone)]
pub enum CompiledQueryMatcherInner {
    Single(CompiledStringMatcher),
    Or(Vec<CompiledStringMatcher>),
}

/// Compiled query parameter matcher.
#[derive(Debug, Clone)]
pub struct CompiledQueryMatcher {
    pub name: String,
    pub matcher: CompiledQueryMatcherInner,
    pub case_sensitive: bool,
    /// Negate the match result (NOT operator)
    pub not: bool,
    /// Optional except pattern for stripping content before matching
    pub except: Option<CompiledExcept>,
}

impl CompiledQueryMatcher {
    /// Compile a QueryMatcher configuration.
    pub fn compile(config: &QueryMatcher) -> Result<Self, regex::Error> {
        match config {
            QueryMatcher::Simple { name, value } => Ok(CompiledQueryMatcher {
                name: name.clone(),
                matcher: CompiledQueryMatcherInner::Single(CompiledStringMatcher::Equals {
                    value: value.clone(),
                    lower: value.to_lowercase(),
                }),
                case_sensitive: true,
                not: false,
                except: None,
            }),
            QueryMatcher::Or { name, or, options } => {
                let compiled: Result<Vec<_>, _> =
                    or.iter().map(CompiledStringMatcher::compile).collect();
                let except = options
                    .except
                    .as_ref()
                    .map(|p| CompiledExcept::compile(p))
                    .transpose()?;
                Ok(CompiledQueryMatcher {
                    name: name.clone(),
                    matcher: CompiledQueryMatcherInner::Or(compiled?),
                    case_sensitive: options.case_sensitive,
                    not: options.not,
                    except,
                })
            }
            QueryMatcher::Full {
                name,
                matcher,
                options,
            } => {
                let except = options
                    .except
                    .as_ref()
                    .map(|p| CompiledExcept::compile(p))
                    .transpose()?;
                Ok(CompiledQueryMatcher {
                    name: name.clone(),
                    matcher: CompiledQueryMatcherInner::Single(CompiledStringMatcher::compile(
                        matcher,
                    )?),
                    case_sensitive: options.case_sensitive,
                    not: options.not,
                    except,
                })
            }
        }
    }

    /// Check if a query parameter value matches.
    pub fn matches(&self, value: Option<&str>) -> bool {
        let result = match &self.matcher {
            CompiledQueryMatcherInner::Single(m) => {
                m.matches_with_except(value, self.case_sensitive, self.except.as_ref())
            }
            CompiledQueryMatcherInner::Or(matchers) => matchers
                .iter()
                .any(|m| m.matches_with_except(value, self.case_sensitive, self.except.as_ref())),
        };
        if self.not {
            !result
        } else {
            result
        }
    }
}

/// Path matching configuration with full predicate support.
///
/// Backward compatible with existing Rift config format while supporting
/// new Mountebank-style predicates.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq)]
#[serde(untagged)]
pub enum PathMatcher {
    /// Match any path (default)
    #[default]
    Any,

    /// Exact path match (backward compatible): { exact: "/api/users" }
    Exact { exact: String },

    /// Prefix match (backward compatible): { prefix: "/api" }
    Prefix { prefix: String },

    /// Regex match (backward compatible): { regex: "^/api/v\\d+/" }
    Regex { regex: String },

    /// Contains substring: { contains: "/api" }
    Contains { contains: String },

    /// Ends with suffix: { endsWith: ".json" }
    EndsWith {
        #[serde(rename = "endsWith")]
        ends_with: String,
    },

    /// Full predicate with options
    Full {
        #[serde(flatten)]
        matcher: StringMatcher,
        #[serde(flatten, default)]
        options: PredicateOptions,
    },
}

/// Compiled path matcher for efficient runtime evaluation.
#[derive(Debug, Clone)]
pub enum CompiledPathMatcher {
    Any,
    Exact { value: String, lower: String },
    Prefix { value: String, lower: String },
    Contains { value: String, lower: String },
    EndsWith { value: String, lower: String },
    Regex(Arc<Regex>),
}

/// Compiled path match configuration including options.
#[derive(Debug, Clone)]
pub struct CompiledPathMatch {
    pub matcher: CompiledPathMatcher,
    pub case_sensitive: bool,
}

impl CompiledPathMatch {
    /// Compile a PathMatcher configuration.
    pub fn compile(config: &PathMatcher) -> Result<Self, regex::Error> {
        match config {
            PathMatcher::Any => Ok(CompiledPathMatch {
                matcher: CompiledPathMatcher::Any,
                case_sensitive: true,
            }),

            PathMatcher::Exact { exact } => Ok(CompiledPathMatch {
                matcher: CompiledPathMatcher::Exact {
                    value: exact.clone(),
                    lower: exact.to_lowercase(),
                },
                case_sensitive: true,
            }),

            PathMatcher::Prefix { prefix } => Ok(CompiledPathMatch {
                matcher: CompiledPathMatcher::Prefix {
                    value: prefix.clone(),
                    lower: prefix.to_lowercase(),
                },
                case_sensitive: true,
            }),

            PathMatcher::Regex { regex } => Ok(CompiledPathMatch {
                matcher: CompiledPathMatcher::Regex(Arc::new(Regex::new(regex)?)),
                case_sensitive: true,
            }),

            PathMatcher::Contains { contains } => Ok(CompiledPathMatch {
                matcher: CompiledPathMatcher::Contains {
                    value: contains.clone(),
                    lower: contains.to_lowercase(),
                },
                case_sensitive: true,
            }),

            PathMatcher::EndsWith { ends_with } => Ok(CompiledPathMatch {
                matcher: CompiledPathMatcher::EndsWith {
                    value: ends_with.clone(),
                    lower: ends_with.to_lowercase(),
                },
                case_sensitive: true,
            }),

            PathMatcher::Full { matcher, options } => {
                let compiled = match matcher {
                    StringMatcher::Equals(v) => CompiledPathMatcher::Exact {
                        value: v.clone(),
                        lower: v.to_lowercase(),
                    },
                    StringMatcher::Contains(v) => CompiledPathMatcher::Contains {
                        value: v.clone(),
                        lower: v.to_lowercase(),
                    },
                    StringMatcher::StartsWith(v) => CompiledPathMatcher::Prefix {
                        value: v.clone(),
                        lower: v.to_lowercase(),
                    },
                    StringMatcher::EndsWith(v) => CompiledPathMatcher::EndsWith {
                        value: v.clone(),
                        lower: v.to_lowercase(),
                    },
                    StringMatcher::Matches(pattern) => {
                        CompiledPathMatcher::Regex(Arc::new(Regex::new(pattern)?))
                    }
                    StringMatcher::Exists(_) => CompiledPathMatcher::Any, // Path always exists
                };

                Ok(CompiledPathMatch {
                    matcher: compiled,
                    case_sensitive: options.case_sensitive,
                })
            }
        }
    }

    /// Check if a path matches this matcher.
    pub fn matches(&self, path: &str) -> bool {
        match &self.matcher {
            CompiledPathMatcher::Any => true,

            CompiledPathMatcher::Exact { value, lower } => {
                if self.case_sensitive {
                    path == value
                } else {
                    path.to_lowercase() == *lower
                }
            }

            CompiledPathMatcher::Prefix { value, lower } => {
                if self.case_sensitive {
                    path.starts_with(value.as_str())
                } else {
                    path.to_lowercase().starts_with(lower.as_str())
                }
            }

            CompiledPathMatcher::Contains { value, lower } => {
                if self.case_sensitive {
                    path.contains(value.as_str())
                } else {
                    path.to_lowercase().contains(lower.as_str())
                }
            }

            CompiledPathMatcher::EndsWith { value, lower } => {
                if self.case_sensitive {
                    path.ends_with(value.as_str())
                } else {
                    path.to_lowercase().ends_with(lower.as_str())
                }
            }

            CompiledPathMatcher::Regex(regex) => regex.is_match(path),
        }
    }
}

/// Deep equality matcher for objects (headers, query params).
///
/// Unlike regular `equals`, `deepEquals` requires an EXACT match:
/// - All specified key-value pairs must be present and equal
/// - NO extra keys are allowed in the actual value
///
/// This is the Mountebank `deepEquals` predicate behavior.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DeepEquals {
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub query: Option<HashMap<String, String>>,
}

/// Compiled deep equality matcher.
#[derive(Debug, Clone)]
pub struct CompiledDeepEquals {
    /// Expected headers (keys lowercased)
    pub headers: Option<HashMap<String, String>>,
    /// Expected query parameters
    pub query: Option<HashMap<String, String>>,
    /// Case sensitive comparison
    pub case_sensitive: bool,
}

impl CompiledDeepEquals {
    /// Compile a DeepEquals configuration.
    pub fn compile(config: &DeepEquals, case_sensitive: bool) -> Self {
        CompiledDeepEquals {
            headers: config.headers.as_ref().map(|h| {
                h.iter()
                    .map(|(k, v)| (k.to_lowercase(), v.clone()))
                    .collect()
            }),
            query: config.query.clone(),
            case_sensitive,
        }
    }

    /// Check if headers match the deep equality constraint (exact match, no extra headers).
    ///
    /// Note: For headers, we only check against the expected headers since HTTP headers
    /// typically include many standard headers. Use `matches_headers_strict` for true deep equality.
    pub fn matches_headers(&self, headers: &hyper::HeaderMap) -> bool {
        if let Some(expected) = &self.headers {
            for (name, expected_value) in expected {
                match headers.get(name.as_str()) {
                    Some(actual) => {
                        let actual_str = actual.to_str().unwrap_or("");
                        let matches = if self.case_sensitive {
                            actual_str == expected_value
                        } else {
                            actual_str.to_lowercase() == expected_value.to_lowercase()
                        };
                        if !matches {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }
        true
    }

    /// Check if query parameters match the deep equality constraint.
    ///
    /// This is a strict deep equality check:
    /// - All expected parameters must be present with matching values
    /// - NO extra parameters are allowed
    pub fn matches_query(&self, query_params: &HashMap<String, String>) -> bool {
        if let Some(expected) = &self.query {
            // Check that all expected params exist with correct values
            for (name, expected_value) in expected {
                match query_params.get(name) {
                    Some(actual) => {
                        let matches = if self.case_sensitive {
                            actual == expected_value
                        } else {
                            actual.to_lowercase() == expected_value.to_lowercase()
                        };
                        if !matches {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
            // Check that NO extra params exist (deepEquals is strict)
            if query_params.len() != expected.len() {
                return false;
            }
        }
        true
    }

    /// Check if query parameters match using partial equality (like regular `equals`).
    ///
    /// Only checks that expected parameters exist with matching values.
    /// Extra parameters are allowed.
    pub fn matches_query_partial(&self, query_params: &HashMap<String, String>) -> bool {
        if let Some(expected) = &self.query {
            for (name, expected_value) in expected {
                match query_params.get(name) {
                    Some(actual) => {
                        let matches = if self.case_sensitive {
                            actual == expected_value
                        } else {
                            actual.to_lowercase() == expected_value.to_lowercase()
                        };
                        if !matches {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }
        true
    }
}

/// Parse query string into a HashMap.
pub fn parse_query_string(query: Option<&str>) -> HashMap<String, String> {
    let mut params = HashMap::new();
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                // URL decode would go here for full compatibility
                params.insert(
                    key.to_string(),
                    urlencoding::decode(value).unwrap_or_default().to_string(),
                );
            } else if !pair.is_empty() {
                params.insert(pair.to_string(), String::new());
            }
        }
    }
    params
}

// ============================================================================
// Logical Operators (NOT, OR, AND)
// ============================================================================

/// Logical predicate for combining multiple string matchers.
///
/// Supports Mountebank's logical operators: not, or, and.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum LogicalMatcher {
    /// Negates the inner matcher
    Not(Box<LogicalMatcher>),

    /// Matches if ANY of the inner matchers match
    Or(Vec<LogicalMatcher>),

    /// Matches if ALL of the inner matchers match
    And(Vec<LogicalMatcher>),

    /// A leaf string matcher
    #[serde(untagged)]
    Leaf(StringMatcher),
}

impl Default for LogicalMatcher {
    fn default() -> Self {
        LogicalMatcher::Leaf(StringMatcher::Exists(true))
    }
}

/// Compiled logical matcher for efficient runtime evaluation.
#[derive(Debug, Clone)]
pub enum CompiledLogicalMatcher {
    Not(Box<CompiledLogicalMatcher>),
    Or(Vec<CompiledLogicalMatcher>),
    And(Vec<CompiledLogicalMatcher>),
    Leaf(CompiledStringMatcher),
}

impl CompiledLogicalMatcher {
    /// Compile a LogicalMatcher configuration.
    pub fn compile(matcher: &LogicalMatcher) -> Result<Self, regex::Error> {
        match matcher {
            LogicalMatcher::Not(inner) => {
                Ok(CompiledLogicalMatcher::Not(Box::new(Self::compile(inner)?)))
            }
            LogicalMatcher::Or(matchers) => {
                let compiled: Result<Vec<_>, _> = matchers.iter().map(Self::compile).collect();
                Ok(CompiledLogicalMatcher::Or(compiled?))
            }
            LogicalMatcher::And(matchers) => {
                let compiled: Result<Vec<_>, _> = matchers.iter().map(Self::compile).collect();
                Ok(CompiledLogicalMatcher::And(compiled?))
            }
            LogicalMatcher::Leaf(string_matcher) => Ok(CompiledLogicalMatcher::Leaf(
                CompiledStringMatcher::compile(string_matcher)?,
            )),
        }
    }

    /// Check if a value matches this logical matcher.
    pub fn matches(&self, value: Option<&str>, case_sensitive: bool) -> bool {
        match self {
            CompiledLogicalMatcher::Not(inner) => !inner.matches(value, case_sensitive),
            CompiledLogicalMatcher::Or(matchers) => {
                matchers.iter().any(|m| m.matches(value, case_sensitive))
            }
            CompiledLogicalMatcher::And(matchers) => {
                matchers.iter().all(|m| m.matches(value, case_sensitive))
            }
            CompiledLogicalMatcher::Leaf(matcher) => matcher.matches(value, case_sensitive),
        }
    }
}

// ============================================================================
// Body Matching
// ============================================================================

/// Body matching configuration.
///
/// Supports various body matching strategies for request body content.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BodyMatcher {
    /// Exact string match
    Equals(String),

    /// String contains substring
    Contains(String),

    /// Regex pattern match
    Matches(String),

    /// JSON deep equality (for JSON bodies)
    #[serde(rename = "jsonEquals")]
    JsonEquals(serde_json::Value),

    /// JSON path expression match
    #[serde(rename = "jsonPath")]
    JsonPath {
        path: String,
        #[serde(flatten)]
        matcher: StringMatcher,
    },

    /// XPath expression match for XML bodies (Mountebank compatibility)
    #[serde(rename = "xpath")]
    XPath {
        path: String,
        #[serde(flatten)]
        matcher: StringMatcher,
    },
}

/// Compiled body matcher for efficient runtime evaluation.
#[derive(Debug, Clone)]
pub enum CompiledBodyMatcher {
    Equals {
        value: String,
        lower: String,
    },
    Contains {
        value: String,
        lower: String,
    },
    Matches(Arc<Regex>),
    JsonEquals(serde_json::Value),
    JsonPath {
        path: String,
        matcher: CompiledStringMatcher,
    },
    XPath {
        path: String,
        matcher: CompiledStringMatcher,
    },
}

impl CompiledBodyMatcher {
    /// Compile a BodyMatcher configuration.
    pub fn compile(matcher: &BodyMatcher) -> Result<Self, regex::Error> {
        match matcher {
            BodyMatcher::Equals(v) => Ok(CompiledBodyMatcher::Equals {
                value: v.clone(),
                lower: v.to_lowercase(),
            }),
            BodyMatcher::Contains(v) => Ok(CompiledBodyMatcher::Contains {
                value: v.clone(),
                lower: v.to_lowercase(),
            }),
            BodyMatcher::Matches(pattern) => {
                Ok(CompiledBodyMatcher::Matches(Arc::new(Regex::new(pattern)?)))
            }
            BodyMatcher::JsonEquals(value) => Ok(CompiledBodyMatcher::JsonEquals(value.clone())),
            BodyMatcher::JsonPath { path, matcher } => Ok(CompiledBodyMatcher::JsonPath {
                path: path.clone(),
                matcher: CompiledStringMatcher::compile(matcher)?,
            }),
            BodyMatcher::XPath { path, matcher } => Ok(CompiledBodyMatcher::XPath {
                path: path.clone(),
                matcher: CompiledStringMatcher::compile(matcher)?,
            }),
        }
    }

    /// Check if a body matches this matcher.
    pub fn matches(&self, body: &str, case_sensitive: bool) -> bool {
        match self {
            CompiledBodyMatcher::Equals { value, lower } => {
                if case_sensitive {
                    body == value
                } else {
                    body.to_lowercase() == *lower
                }
            }
            CompiledBodyMatcher::Contains { value, lower } => {
                if case_sensitive {
                    body.contains(value.as_str())
                } else {
                    body.to_lowercase().contains(lower.as_str())
                }
            }
            CompiledBodyMatcher::Matches(regex) => regex.is_match(body),
            CompiledBodyMatcher::JsonEquals(expected) => {
                // Parse body as JSON and compare
                match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(actual) => json_deep_equals(&actual, expected, case_sensitive),
                    Err(_) => false,
                }
            }
            CompiledBodyMatcher::JsonPath { path, matcher } => {
                // Simple JSONPath implementation for common patterns
                match extract_json_path(body, path) {
                    Some(value) => matcher.matches(Some(&value), case_sensitive),
                    None => matcher.matches(None, case_sensitive),
                }
            }
            CompiledBodyMatcher::XPath { path, matcher } => {
                // XPath extraction for XML bodies
                match extract_xpath(body, path) {
                    Some(value) => matcher.matches(Some(&value), case_sensitive),
                    None => matcher.matches(None, case_sensitive),
                }
            }
        }
    }
}

/// Deep JSON equality comparison with optional case sensitivity.
fn json_deep_equals(
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    case_sensitive: bool,
) -> bool {
    use serde_json::Value;

    match (actual, expected) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Number(a), Value::Number(b)) => a == b,
        (Value::String(a), Value::String(b)) => {
            if case_sensitive {
                a == b
            } else {
                a.to_lowercase() == b.to_lowercase()
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| json_deep_equals(x, y, case_sensitive))
        }
        (Value::Object(a), Value::Object(b)) => {
            // All expected keys must be present and match
            b.iter().all(|(key, expected_val)| {
                a.get(key).is_some_and(|actual_val| {
                    json_deep_equals(actual_val, expected_val, case_sensitive)
                })
            })
        }
        _ => false,
    }
}

/// Extract a value from JSON using a simple JSONPath expression.
///
/// Supports:
/// - `$.field` - top-level field
/// - `$.field.nested` - nested field
/// - `$.array[0]` - array index
/// - `$.array[*].field` - all elements' field (returns first match)
fn extract_json_path(body: &str, path: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(body).ok()?;

    // Remove leading $. if present
    let path = path.strip_prefix("$.").unwrap_or(path);
    let path = path.strip_prefix('$').unwrap_or(path);

    let value = navigate_json(&json, path)?;

    match value {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        serde_json::Value::Null => Some("null".to_string()),
        _ => Some(value.to_string()),
    }
}

/// Navigate JSON structure following a path.
fn navigate_json<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    if path.is_empty() {
        return Some(value);
    }

    // Split on first . or [
    let (segment, rest) = if let Some(bracket_pos) = path.find('[') {
        let dot_pos = path.find('.');
        match dot_pos {
            Some(d) if d < bracket_pos => {
                let (seg, rest) = path.split_at(d);
                (seg, rest.strip_prefix('.').unwrap_or(rest))
            }
            _ => {
                let (seg, rest) = path.split_at(bracket_pos);
                (seg, rest)
            }
        }
    } else if let Some(dot_pos) = path.find('.') {
        let (seg, rest) = path.split_at(dot_pos);
        (seg, rest.strip_prefix('.').unwrap_or(rest))
    } else {
        (path, "")
    };

    // Handle array index
    if segment.is_empty() && path.starts_with('[') {
        if let Some(end) = path.find(']') {
            let index_str = &path[1..end];
            let rest = path[end + 1..]
                .strip_prefix('.')
                .unwrap_or(&path[end + 1..]);

            if index_str == "*" {
                // Wildcard - return first match from array
                if let serde_json::Value::Array(arr) = value {
                    for item in arr {
                        if let Some(result) = navigate_json(item, rest) {
                            return Some(result);
                        }
                    }
                }
                return None;
            } else if let Ok(index) = index_str.parse::<usize>() {
                let arr = value.as_array()?;
                let item = arr.get(index)?;
                return navigate_json(item, rest);
            }
        }
        return None;
    }

    // Handle object field
    let obj = value.as_object()?;
    let next = obj.get(segment)?;
    navigate_json(next, rest)
}

/// Extract a value from XML using an XPath expression.
///
/// Supports common XPath patterns:
/// - `/root/element` - absolute path
/// - `//element` - descendant search
/// - `/root/element/@attribute` - attribute selection
/// - `/root/element/text()` - text content
fn extract_xpath(body: &str, path: &str) -> Option<String> {
    use sxd_document::parser;
    use sxd_xpath::{evaluate_xpath, Value};

    // Parse the XML document
    let package = parser::parse(body).ok()?;
    let document = package.as_document();

    // Evaluate the XPath expression
    match evaluate_xpath(&document, path) {
        Ok(value) => match value {
            Value::String(s) => Some(s),
            Value::Number(n) => {
                // Format number without unnecessary decimal places
                if n.fract() == 0.0 {
                    Some(format!("{}", n as i64))
                } else {
                    Some(n.to_string())
                }
            }
            Value::Boolean(b) => Some(b.to_string()),
            Value::Nodeset(nodes) => {
                // Return the text content of the first node
                nodes.iter().next().map(|node| node.string_value())
            }
        },
        Err(_) => None,
    }
}

// ============================================================================
// Unified Request Predicate
// ============================================================================

/// A complete request predicate that can match against various request fields.
///
/// This is the main predicate type used in rule matching, combining all
/// supported matching capabilities.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RequestPredicate {
    /// HTTP method match
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<StringMatcher>,

    /// Path match
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathMatcher>,

    /// Header matchers (all must match)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<HeaderMatcher>,

    /// Query parameter matchers (all must match)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub query: Vec<QueryMatcher>,

    /// Body matcher
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<BodyMatcher>,

    /// Global predicate options
    #[serde(flatten, default)]
    pub options: PredicateOptions,
}

/// Compiled request predicate for efficient runtime evaluation.
#[derive(Debug, Clone)]
pub struct CompiledRequestPredicate {
    pub method: Option<CompiledStringMatcher>,
    pub path: Option<CompiledPathMatch>,
    pub headers: Vec<CompiledHeaderMatcher>,
    pub query: Vec<CompiledQueryMatcher>,
    pub body: Option<CompiledBodyMatcher>,
    pub case_sensitive: bool,
}

impl CompiledRequestPredicate {
    /// Compile a RequestPredicate configuration.
    pub fn compile(predicate: &RequestPredicate) -> Result<Self, regex::Error> {
        let method = predicate
            .method
            .as_ref()
            .map(CompiledStringMatcher::compile)
            .transpose()?;

        let path = predicate
            .path
            .as_ref()
            .map(CompiledPathMatch::compile)
            .transpose()?;

        let headers: Result<Vec<_>, _> = predicate
            .headers
            .iter()
            .map(CompiledHeaderMatcher::compile)
            .collect();

        let query: Result<Vec<_>, _> = predicate
            .query
            .iter()
            .map(CompiledQueryMatcher::compile)
            .collect();

        let body = predicate
            .body
            .as_ref()
            .map(CompiledBodyMatcher::compile)
            .transpose()?;

        Ok(CompiledRequestPredicate {
            method,
            path,
            headers: headers?,
            query: query?,
            body,
            case_sensitive: predicate.options.case_sensitive,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_matcher_equals() {
        let matcher =
            CompiledStringMatcher::compile(&StringMatcher::Equals("test".to_string())).unwrap();

        assert!(matcher.matches(Some("test"), true));
        assert!(!matcher.matches(Some("TEST"), true));
        assert!(matcher.matches(Some("TEST"), false));
        assert!(!matcher.matches(Some("other"), true));
        assert!(!matcher.matches(None, true));
    }

    #[test]
    fn test_string_matcher_contains() {
        let matcher =
            CompiledStringMatcher::compile(&StringMatcher::Contains("api".to_string())).unwrap();

        assert!(matcher.matches(Some("/api/v1"), true));
        assert!(matcher.matches(Some("my-api-service"), true));
        assert!(!matcher.matches(Some("/API/v1"), true));
        assert!(matcher.matches(Some("/API/v1"), false));
        assert!(!matcher.matches(Some("other"), true));
        assert!(!matcher.matches(None, true));
    }

    #[test]
    fn test_string_matcher_starts_with() {
        let matcher =
            CompiledStringMatcher::compile(&StringMatcher::StartsWith("/api".to_string())).unwrap();

        assert!(matcher.matches(Some("/api/v1"), true));
        assert!(matcher.matches(Some("/api"), true));
        assert!(!matcher.matches(Some("/API/v1"), true));
        assert!(matcher.matches(Some("/API/v1"), false));
        assert!(!matcher.matches(Some("other/api"), true));
        assert!(!matcher.matches(None, true));
    }

    #[test]
    fn test_string_matcher_ends_with() {
        let matcher =
            CompiledStringMatcher::compile(&StringMatcher::EndsWith(".json".to_string())).unwrap();

        assert!(matcher.matches(Some("/data.json"), true));
        assert!(matcher.matches(Some(".json"), true));
        assert!(!matcher.matches(Some("/data.JSON"), true));
        assert!(matcher.matches(Some("/data.JSON"), false));
        assert!(!matcher.matches(Some("/data.xml"), true));
        assert!(!matcher.matches(None, true));
    }

    #[test]
    fn test_string_matcher_regex() {
        let matcher =
            CompiledStringMatcher::compile(&StringMatcher::Matches(r"^/api/v\d+/".to_string()))
                .unwrap();

        assert!(matcher.matches(Some("/api/v1/users"), true));
        assert!(matcher.matches(Some("/api/v99/items"), true));
        assert!(!matcher.matches(Some("/api/users"), true));
        assert!(!matcher.matches(None, true));
    }

    #[test]
    fn test_string_matcher_exists() {
        let exists_true = CompiledStringMatcher::compile(&StringMatcher::Exists(true)).unwrap();
        let exists_false = CompiledStringMatcher::compile(&StringMatcher::Exists(false)).unwrap();

        assert!(exists_true.matches(Some("any value"), true));
        assert!(exists_true.matches(Some(""), true));
        assert!(!exists_true.matches(None, true));

        assert!(!exists_false.matches(Some("any value"), true));
        assert!(exists_false.matches(None, true));
    }

    #[test]
    fn test_path_matcher_backward_compatible() {
        // Test existing Rift config format works
        let exact = CompiledPathMatch::compile(&PathMatcher::Exact {
            exact: "/api/users".to_string(),
        })
        .unwrap();
        assert!(exact.matches("/api/users"));
        assert!(!exact.matches("/api/users/1"));

        let prefix = CompiledPathMatch::compile(&PathMatcher::Prefix {
            prefix: "/api".to_string(),
        })
        .unwrap();
        assert!(prefix.matches("/api"));
        assert!(prefix.matches("/api/users"));
        assert!(!prefix.matches("/other"));

        let regex = CompiledPathMatch::compile(&PathMatcher::Regex {
            regex: r"^/api/v\d+/.*".to_string(),
        })
        .unwrap();
        assert!(regex.matches("/api/v1/users"));
        assert!(!regex.matches("/api/users"));
    }

    #[test]
    fn test_path_matcher_new_operators() {
        let contains = CompiledPathMatch::compile(&PathMatcher::Contains {
            contains: "users".to_string(),
        })
        .unwrap();
        assert!(contains.matches("/api/users"));
        assert!(contains.matches("/users/list"));
        assert!(!contains.matches("/api/items"));

        let ends_with = CompiledPathMatch::compile(&PathMatcher::EndsWith {
            ends_with: ".json".to_string(),
        })
        .unwrap();
        assert!(ends_with.matches("/data.json"));
        assert!(!ends_with.matches("/data.xml"));
    }

    #[test]
    fn test_header_matcher_simple() {
        let config = HeaderMatcher::Simple {
            name: "X-Api-Key".to_string(),
            value: "secret".to_string(),
        };
        let compiled = CompiledHeaderMatcher::compile(&config).unwrap();

        assert_eq!(compiled.name, "x-api-key"); // Lowercased
        assert!(compiled.matches(Some("secret")));
        assert!(!compiled.matches(Some("other")));
        assert!(!compiled.matches(None));
    }

    #[test]
    fn test_query_string_parsing() {
        let params = parse_query_string(Some("page=1&sort=desc&filter=active"));
        assert_eq!(params.get("page"), Some(&"1".to_string()));
        assert_eq!(params.get("sort"), Some(&"desc".to_string()));
        assert_eq!(params.get("filter"), Some(&"active".to_string()));

        let empty = parse_query_string(None);
        assert!(empty.is_empty());

        let encoded = parse_query_string(Some("name=hello%20world"));
        assert_eq!(encoded.get("name"), Some(&"hello world".to_string()));
    }

    #[test]
    fn test_deep_equals_headers() {
        use hyper::header::{HeaderName, HeaderValue};
        use hyper::HeaderMap;

        let config = DeepEquals {
            headers: Some(
                [("x-api-key".to_string(), "secret".to_string())]
                    .into_iter()
                    .collect(),
            ),
            query: None,
        };
        let compiled = CompiledDeepEquals::compile(&config, true);

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("secret"),
        );
        assert!(compiled.matches_headers(&headers));

        let mut wrong_headers = HeaderMap::new();
        wrong_headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_static("wrong"),
        );
        assert!(!compiled.matches_headers(&wrong_headers));

        let empty_headers = HeaderMap::new();
        assert!(!compiled.matches_headers(&empty_headers));
    }

    // ========================================================================
    // Logical Operator Tests
    // ========================================================================

    #[test]
    fn test_logical_not() {
        let matcher = CompiledLogicalMatcher::compile(&LogicalMatcher::Not(Box::new(
            LogicalMatcher::Leaf(StringMatcher::Equals("test".to_string())),
        )))
        .unwrap();

        assert!(!matcher.matches(Some("test"), true)); // NOT equals "test"
        assert!(matcher.matches(Some("other"), true)); // NOT equals "other" -> true
        assert!(matcher.matches(None, true)); // NOT exists -> true
    }

    #[test]
    fn test_logical_or() {
        let matcher = CompiledLogicalMatcher::compile(&LogicalMatcher::Or(vec![
            LogicalMatcher::Leaf(StringMatcher::Equals("foo".to_string())),
            LogicalMatcher::Leaf(StringMatcher::Equals("bar".to_string())),
            LogicalMatcher::Leaf(StringMatcher::Equals("baz".to_string())),
        ]))
        .unwrap();

        assert!(matcher.matches(Some("foo"), true));
        assert!(matcher.matches(Some("bar"), true));
        assert!(matcher.matches(Some("baz"), true));
        assert!(!matcher.matches(Some("qux"), true));
        assert!(!matcher.matches(None, true));
    }

    #[test]
    fn test_logical_and() {
        let matcher = CompiledLogicalMatcher::compile(&LogicalMatcher::And(vec![
            LogicalMatcher::Leaf(StringMatcher::Contains("api".to_string())),
            LogicalMatcher::Leaf(StringMatcher::StartsWith("/".to_string())),
        ]))
        .unwrap();

        assert!(matcher.matches(Some("/api/v1"), true));
        assert!(matcher.matches(Some("/my-api"), true));
        assert!(!matcher.matches(Some("api/v1"), true)); // Doesn't start with /
        assert!(!matcher.matches(Some("/users"), true)); // Doesn't contain api
    }

    #[test]
    fn test_logical_nested() {
        // NOT (foo OR bar) - should match anything except "foo" or "bar"
        let matcher = CompiledLogicalMatcher::compile(&LogicalMatcher::Not(Box::new(
            LogicalMatcher::Or(vec![
                LogicalMatcher::Leaf(StringMatcher::Equals("foo".to_string())),
                LogicalMatcher::Leaf(StringMatcher::Equals("bar".to_string())),
            ]),
        )))
        .unwrap();

        assert!(!matcher.matches(Some("foo"), true));
        assert!(!matcher.matches(Some("bar"), true));
        assert!(matcher.matches(Some("baz"), true));
        assert!(matcher.matches(Some("anything"), true));
    }

    // ========================================================================
    // Body Matcher Tests
    // ========================================================================

    #[test]
    fn test_body_matcher_equals() {
        let matcher =
            CompiledBodyMatcher::compile(&BodyMatcher::Equals("hello world".to_string())).unwrap();

        assert!(matcher.matches("hello world", true));
        assert!(!matcher.matches("HELLO WORLD", true));
        assert!(matcher.matches("HELLO WORLD", false));
        assert!(!matcher.matches("hello", true));
    }

    #[test]
    fn test_body_matcher_contains() {
        let matcher =
            CompiledBodyMatcher::compile(&BodyMatcher::Contains("api".to_string())).unwrap();

        assert!(matcher.matches("this is an api call", true));
        assert!(!matcher.matches("this is an API call", true));
        assert!(matcher.matches("this is an API call", false));
        assert!(!matcher.matches("no match here", true));
    }

    #[test]
    fn test_body_matcher_regex() {
        let matcher =
            CompiledBodyMatcher::compile(&BodyMatcher::Matches(r"\d{3}-\d{4}".to_string()))
                .unwrap();

        assert!(matcher.matches("Call me at 123-4567", true));
        assert!(matcher.matches("Phone: 999-0000", true));
        assert!(!matcher.matches("No phone number", true));
    }

    #[test]
    fn test_body_matcher_json_equals() {
        let expected = serde_json::json!({
            "name": "John",
            "age": 30
        });
        let matcher = CompiledBodyMatcher::compile(&BodyMatcher::JsonEquals(expected)).unwrap();

        // Exact match
        assert!(matcher.matches(r#"{"name": "John", "age": 30}"#, true));

        // Order doesn't matter
        assert!(matcher.matches(r#"{"age": 30, "name": "John"}"#, true));

        // Extra fields in actual are OK (partial match)
        assert!(matcher.matches(r#"{"name": "John", "age": 30, "city": "NYC"}"#, true));

        // Missing fields fail
        assert!(!matcher.matches(r#"{"name": "John"}"#, true));

        // Wrong values fail
        assert!(!matcher.matches(r#"{"name": "Jane", "age": 30}"#, true));

        // Case insensitive string comparison
        assert!(!matcher.matches(r#"{"name": "JOHN", "age": 30}"#, true));
        assert!(matcher.matches(r#"{"name": "JOHN", "age": 30}"#, false));
    }

    #[test]
    fn test_body_matcher_json_path() {
        let matcher = CompiledBodyMatcher::compile(&BodyMatcher::JsonPath {
            path: "$.user.name".to_string(),
            matcher: StringMatcher::Equals("John".to_string()),
        })
        .unwrap();

        assert!(matcher.matches(r#"{"user": {"name": "John", "age": 30}}"#, true));
        assert!(!matcher.matches(r#"{"user": {"name": "Jane", "age": 25}}"#, true));
        assert!(!matcher.matches(r#"{"user": {"age": 30}}"#, true));
    }

    // ========================================================================
    // JSON Path Navigation Tests
    // ========================================================================

    #[test]
    fn test_json_path_simple_field() {
        let body = r#"{"name": "John", "age": 30}"#;
        assert_eq!(extract_json_path(body, "$.name"), Some("John".to_string()));
        assert_eq!(extract_json_path(body, "$.age"), Some("30".to_string()));
        assert_eq!(extract_json_path(body, "$.missing"), None);
    }

    #[test]
    fn test_json_path_nested() {
        let body = r#"{"user": {"profile": {"name": "John"}}}"#;
        assert_eq!(
            extract_json_path(body, "$.user.profile.name"),
            Some("John".to_string())
        );
    }

    #[test]
    fn test_json_path_array_index() {
        let body = r#"{"users": [{"name": "Alice"}, {"name": "Bob"}]}"#;
        assert_eq!(
            extract_json_path(body, "$.users[0].name"),
            Some("Alice".to_string())
        );
        assert_eq!(
            extract_json_path(body, "$.users[1].name"),
            Some("Bob".to_string())
        );
        assert_eq!(extract_json_path(body, "$.users[2].name"), None);
    }

    #[test]
    fn test_json_path_wildcard() {
        let body = r#"{"items": [{"id": 1}, {"id": 2}, {"id": 3}]}"#;
        // Wildcard returns first match
        assert_eq!(
            extract_json_path(body, "$.items[*].id"),
            Some("1".to_string())
        );
    }

    // ========================================================================
    // Request Predicate Tests
    // ========================================================================

    #[test]
    fn test_request_predicate_compile() {
        let predicate = RequestPredicate {
            method: Some(StringMatcher::Equals("GET".to_string())),
            path: Some(PathMatcher::Prefix {
                prefix: "/api".to_string(),
            }),
            headers: vec![HeaderMatcher::Simple {
                name: "Content-Type".to_string(),
                value: "application/json".to_string(),
            }],
            query: vec![QueryMatcher::Simple {
                name: "page".to_string(),
                value: "1".to_string(),
            }],
            body: None,
            options: PredicateOptions::default(),
        };

        let compiled = CompiledRequestPredicate::compile(&predicate);
        assert!(compiled.is_ok());

        let compiled = compiled.unwrap();
        assert!(compiled.method.is_some());
        assert!(compiled.path.is_some());
        assert_eq!(compiled.headers.len(), 1);
        assert_eq!(compiled.query.len(), 1);
    }

    // ========================================================================
    // Serde Serialization Tests
    // ========================================================================

    #[test]
    fn test_string_matcher_serde() {
        // Test equals
        let json = r#"{"equals": "test"}"#;
        let matcher: StringMatcher = serde_json::from_str(json).unwrap();
        assert_eq!(matcher, StringMatcher::Equals("test".to_string()));

        // Test contains
        let json = r#"{"contains": "api"}"#;
        let matcher: StringMatcher = serde_json::from_str(json).unwrap();
        assert_eq!(matcher, StringMatcher::Contains("api".to_string()));

        // Test startsWith
        let json = r#"{"startsWith": "/api"}"#;
        let matcher: StringMatcher = serde_json::from_str(json).unwrap();
        assert_eq!(matcher, StringMatcher::StartsWith("/api".to_string()));

        // Test endsWith
        let json = r#"{"endsWith": ".json"}"#;
        let matcher: StringMatcher = serde_json::from_str(json).unwrap();
        assert_eq!(matcher, StringMatcher::EndsWith(".json".to_string()));

        // Test matches (regex)
        let json = r#"{"matches": "^/api/v\\d+"}"#;
        let matcher: StringMatcher = serde_json::from_str(json).unwrap();
        assert_eq!(matcher, StringMatcher::Matches(r"^/api/v\d+".to_string()));

        // Test exists
        let json = r#"{"exists": true}"#;
        let matcher: StringMatcher = serde_json::from_str(json).unwrap();
        assert_eq!(matcher, StringMatcher::Exists(true));
    }

    #[test]
    fn test_header_matcher_serde() {
        // Simple format (backward compatible)
        let json = r#"{"name": "X-Api-Key", "value": "secret"}"#;
        let matcher: HeaderMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, HeaderMatcher::Simple { .. }));

        // Full format with operators
        let json = r#"{"name": "Content-Type", "contains": "json"}"#;
        let matcher: HeaderMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, HeaderMatcher::Full { .. }));
    }

    #[test]
    fn test_path_matcher_serde() {
        // Exact path (backward compatible)
        let json = r#"{"exact": "/api/users"}"#;
        let matcher: PathMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, PathMatcher::Exact { .. }));

        // Prefix path (backward compatible)
        let json = r#"{"prefix": "/api"}"#;
        let matcher: PathMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, PathMatcher::Prefix { .. }));

        // Regex path (backward compatible)
        let json = r#"{"regex": "^/api/v\\d+"}"#;
        let matcher: PathMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, PathMatcher::Regex { .. }));

        // New contains
        let json = r#"{"contains": "users"}"#;
        let matcher: PathMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, PathMatcher::Contains { .. }));

        // New endsWith
        let json = r#"{"endsWith": ".json"}"#;
        let matcher: PathMatcher = serde_json::from_str(json).unwrap();
        assert!(matches!(matcher, PathMatcher::EndsWith { .. }));
    }

    #[test]
    fn test_predicate_options_default() {
        let options = PredicateOptions::default();
        assert!(options.case_sensitive); // Rift default is case-sensitive
        assert!(options.except.is_none());
    }

    // ========================================================================
    // DeepEquals Strict Matching Tests
    // ========================================================================

    #[test]
    fn test_deep_equals_query_strict() {
        let config = DeepEquals {
            headers: None,
            query: Some(
                [
                    ("page".to_string(), "1".to_string()),
                    ("sort".to_string(), "desc".to_string()),
                ]
                .into_iter()
                .collect(),
            ),
        };
        let compiled = CompiledDeepEquals::compile(&config, true);

        // Exact match - should pass
        let exact: HashMap<String, String> = [
            ("page".to_string(), "1".to_string()),
            ("sort".to_string(), "desc".to_string()),
        ]
        .into_iter()
        .collect();
        assert!(compiled.matches_query(&exact));

        // Missing param - should fail
        let missing: HashMap<String, String> = [("page".to_string(), "1".to_string())]
            .into_iter()
            .collect();
        assert!(!compiled.matches_query(&missing));

        // Extra param - should fail (deepEquals is strict)
        let extra: HashMap<String, String> = [
            ("page".to_string(), "1".to_string()),
            ("sort".to_string(), "desc".to_string()),
            ("filter".to_string(), "active".to_string()),
        ]
        .into_iter()
        .collect();
        assert!(!compiled.matches_query(&extra));
    }

    #[test]
    fn test_deep_equals_query_partial() {
        let config = DeepEquals {
            headers: None,
            query: Some(
                [("page".to_string(), "1".to_string())]
                    .into_iter()
                    .collect(),
            ),
        };
        let compiled = CompiledDeepEquals::compile(&config, true);

        // Extra params are allowed with partial matching
        let with_extra: HashMap<String, String> = [
            ("page".to_string(), "1".to_string()),
            ("sort".to_string(), "desc".to_string()),
        ]
        .into_iter()
        .collect();
        assert!(compiled.matches_query_partial(&with_extra));
    }

    // ========================================================================
    // Except Parameter Tests
    // ========================================================================

    #[test]
    fn test_except_parameter() {
        let except = CompiledExcept::compile(r"\d+").unwrap();

        // Strips all digits
        assert_eq!(except.apply("abc123def456"), "abcdef");
        assert_eq!(except.apply("12345"), "");
        assert_eq!(except.apply("no-digits-here"), "no-digits-here");
    }

    #[test]
    fn test_string_matcher_with_except() {
        let matcher =
            CompiledStringMatcher::compile(&StringMatcher::Equals("Hello World".to_string()))
                .unwrap();
        let except = CompiledExcept::compile(r"\d+").unwrap();

        // Without except - doesn't match
        assert!(!matcher.matches(Some("Hello123 World456"), true));

        // With except - strips digits and matches
        assert!(matcher.matches_with_except(Some("Hello123 World456"), true, Some(&except)));
    }

    #[test]
    fn test_header_matcher_with_except() {
        // Header with except - strips version numbers and matches
        // After stripping "5.0" and "89.0", "Mozilla/5.0 Firefox/89.0" becomes "Mozilla/ Firefox/"
        let config = HeaderMatcher::Full {
            name: "user-agent".to_string(),
            matcher: StringMatcher::Equals("Mozilla/ Firefox/".to_string()),
            options: PredicateOptions {
                case_sensitive: true,
                except: Some(r"\d+\.\d+".to_string()), // Strip version numbers
                not: false,
            },
        };

        let compiled = CompiledHeaderMatcher::compile(&config).unwrap();
        // User-Agent with version stripped should match
        assert!(compiled.matches(Some("Mozilla/5.0 Firefox/89.0")));
        assert!(compiled.matches(Some("Mozilla/6.0 Firefox/90.0")));
    }

    // ========================================================================
    // XPath Tests
    // ========================================================================

    #[test]
    fn test_xpath_simple_element() {
        let xml = r#"<root><name>John</name><age>30</age></root>"#;
        assert_eq!(extract_xpath(xml, "/root/name"), Some("John".to_string()));
        assert_eq!(extract_xpath(xml, "/root/age"), Some("30".to_string()));
        assert_eq!(extract_xpath(xml, "/root/missing"), None);
    }

    #[test]
    fn test_xpath_nested() {
        let xml = r#"<root><user><profile><name>Jane</name></profile></user></root>"#;
        assert_eq!(
            extract_xpath(xml, "/root/user/profile/name"),
            Some("Jane".to_string())
        );
    }

    #[test]
    fn test_xpath_attribute() {
        let xml = r#"<root><item id="123">Content</item></root>"#;
        assert_eq!(
            extract_xpath(xml, "/root/item/@id"),
            Some("123".to_string())
        );
    }

    #[test]
    fn test_xpath_descendant() {
        let xml = r#"<root><level1><level2><target>Found</target></level2></level1></root>"#;
        assert_eq!(extract_xpath(xml, "//target"), Some("Found".to_string()));
    }

    #[test]
    fn test_body_matcher_xpath() {
        let matcher = CompiledBodyMatcher::compile(&BodyMatcher::XPath {
            path: "/order/customer/name".to_string(),
            matcher: StringMatcher::Equals("Alice".to_string()),
        })
        .unwrap();

        let xml = r#"<order><customer><name>Alice</name><email>alice@example.com</email></customer></order>"#;
        assert!(matcher.matches(xml, true));

        let xml_wrong = r#"<order><customer><name>Bob</name></customer></order>"#;
        assert!(!matcher.matches(xml_wrong, true));
    }

    #[test]
    fn test_xpath_invalid_xml() {
        let invalid = "not xml at all";
        assert_eq!(extract_xpath(invalid, "/root/name"), None);
    }
}
