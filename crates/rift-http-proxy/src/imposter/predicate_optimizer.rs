//! Predicate optimizer: converts Mountebank predicates to optimized per-field format.
//!
//! This module provides conversion from the Mountebank predicate format (organized per-type)
//! to our optimized per-field format that enables better cache locality and RegexSet optimization.

use super::optimized_predicates::{
    FieldPredicate, MaybeSensitiveStr, OptimizedPredicates, StringPredicate, ValueSelector,
};
use super::types::{Predicate, PredicateOperation, PredicateSelector};
use regex::{Regex, RegexSet};
use std::collections::HashMap;

/// A builder for constructing StringPredicates from multiple predicate operations.
#[derive(Debug, Default)]
struct StringPredicateBuilder {
    /// starts_with pattern (can only have one)
    starts_with: Option<(String, bool)>, // (pattern, case_sensitive)
    /// ends_with pattern (can only have one)
    ends_with: Option<(String, bool)>,
    /// contains patterns (can have multiple)
    contains: Vec<(String, bool)>,
    /// equals pattern (can only have one)
    equals: Option<(String, bool)>,
    /// regex patterns (can have multiple)
    regexes: Vec<String>,
}

impl StringPredicateBuilder {
    fn new() -> Self {
        Self::default()
    }

    fn add_starts_with(&mut self, pattern: String, case_sensitive: bool) {
        self.starts_with = Some((pattern, case_sensitive));
    }

    fn add_ends_with(&mut self, pattern: String, case_sensitive: bool) {
        self.ends_with = Some((pattern, case_sensitive));
    }

    fn add_contains(&mut self, pattern: String, case_sensitive: bool) {
        self.contains.push((pattern, case_sensitive));
    }

    fn add_equals(&mut self, pattern: String, case_sensitive: bool) {
        self.equals = Some((pattern, case_sensitive));
    }

    fn add_regex(&mut self, pattern: String) {
        self.regexes.push(pattern);
    }

    /// Build the final StringPredicate.
    ///
    /// This method chooses the optimal representation based on what operations were added.
    fn build(self) -> Result<StringPredicate, regex::Error> {
        let has_simple = self.starts_with.is_some()
            || self.ends_with.is_some()
            || !self.contains.is_empty()
            || self.equals.is_some();
        let has_regexes = !self.regexes.is_empty();

        match (has_simple, has_regexes) {
            (false, false) => {
                // No constraints at all
                Ok(StringPredicate::empty_simple())
            }
            (true, false) => {
                // Only simple operations
                let mut pred = StringPredicate::empty_simple();

                if let Some((pattern, case_sensitive)) = self.starts_with {
                    pred = pred.with_starts_with(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                if let Some((pattern, case_sensitive)) = self.ends_with {
                    pred = pred.with_ends_with(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                for (pattern, case_sensitive) in self.contains {
                    pred = pred.with_contains(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                if let Some((pattern, case_sensitive)) = self.equals {
                    pred = pred.with_equals(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                Ok(pred)
            }
            (false, true) => {
                // Only regexes - use RegexSet
                let set = RegexSet::new(&self.regexes)?;
                Ok(StringPredicate::Regexes {
                    set,
                    require_all: true, // All regexes must match (AND)
                })
            }
            (true, true) => {
                // Both simple and regexes - use Combined
                let mut simple = StringPredicate::empty_simple();

                if let Some((pattern, case_sensitive)) = self.starts_with {
                    simple = simple.with_starts_with(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                if let Some((pattern, case_sensitive)) = self.ends_with {
                    simple = simple.with_ends_with(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                for (pattern, case_sensitive) in self.contains {
                    simple = simple.with_contains(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                if let Some((pattern, case_sensitive)) = self.equals {
                    simple = simple.with_equals(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                let regexes = RegexSet::new(&self.regexes)?;

                Ok(StringPredicate::Combined {
                    simple: Box::new(simple),
                    regexes,
                    require_all_regexes: true,
                })
            }
        }
    }
}

/// Builder for a FieldPredicate (StringPredicate + except pattern).
#[derive(Debug, Default)]
struct FieldPredicateBuilder {
    string_pred: StringPredicateBuilder,
    except_pattern: Option<String>,
}

impl FieldPredicateBuilder {
    fn build(self) -> Result<FieldPredicate, regex::Error> {
        let pred = self.string_pred.build()?;
        match self.except_pattern {
            Some(pattern) => {
                let except_regex = Regex::new(&pattern)?;
                Ok(FieldPredicate::with_except(pred, except_regex))
            }
            None => Ok(FieldPredicate::new(pred)),
        }
    }

    fn is_empty(&self) -> bool {
        self.string_pred.starts_with.is_none()
            && self.string_pred.ends_with.is_none()
            && self.string_pred.contains.is_empty()
            && self.string_pred.equals.is_none()
            && self.string_pred.regexes.is_empty()
            && self.except_pattern.is_none()
    }
}

/// Per-field builders for constructing optimized predicates.
#[derive(Debug, Default)]
struct FieldBuilders {
    method: FieldPredicateBuilder,
    path: FieldPredicateBuilder,
    body: FieldPredicateBuilder,
    body_selector: Option<PredicateSelector>,
    query: HashMap<String, FieldPredicateBuilder>,
    headers: HashMap<String, FieldPredicateBuilder>,
    request_from: FieldPredicateBuilder,
    ip: FieldPredicateBuilder,
    form: HashMap<String, FieldPredicateBuilder>,
}

/// Convert Mountebank predicates to optimized per-field format.
///
/// This function analyzes all predicates and groups operations by field,
/// enabling optimizations like RegexSet for multiple regex patterns on the same field.
pub fn optimize_predicates(predicates: &[Predicate]) -> Result<OptimizedPredicates, regex::Error> {
    let mut builders = FieldBuilders::default();

    // Process each predicate and add to appropriate field builders
    for predicate in predicates {
        let case_sensitive = predicate.parameters.case_sensitive.unwrap_or(false);
        let except_pattern = if predicate.parameters.except.is_empty() {
            None
        } else {
            Some(predicate.parameters.except.clone())
        };

        // Store selector if present (only applicable to body field)
        if let Some(ref selector) = predicate.parameters.selector {
            builders.body_selector = Some(selector.clone());
        }

        process_predicate_operation(
            &predicate.operation,
            case_sensitive,
            except_pattern,
            &mut builders,
        )?;
    }

    // Build final optimized predicates
    Ok(OptimizedPredicates {
        method: if !builders.method.is_empty() {
            Some(builders.method.build()?)
        } else {
            None
        },
        path: if !builders.path.is_empty() {
            Some(builders.path.build()?)
        } else {
            None
        },
        body: if !builders.body.is_empty() {
            Some(builders.body.build()?)
        } else {
            None
        },
        body_selector: builders.body_selector.map(|s| match s {
            PredicateSelector::JsonPath { selector } => ValueSelector::JsonPath(selector),
            PredicateSelector::XPath {
                selector,
                namespaces,
            } => ValueSelector::XPath {
                selector,
                namespaces,
            },
        }),
        query: builders
            .query
            .into_iter()
            .map(|(k, v)| Ok((k, v.build()?)))
            .collect::<Result<Vec<_>, regex::Error>>()?,
        headers: builders
            .headers
            .into_iter()
            .map(|(k, v)| Ok((k, v.build()?)))
            .collect::<Result<Vec<_>, regex::Error>>()?,
        request_from: if !builders.request_from.is_empty() {
            Some(builders.request_from.build()?)
        } else {
            None
        },
        ip: if !builders.ip.is_empty() {
            Some(builders.ip.build()?)
        } else {
            None
        },
        form: builders
            .form
            .into_iter()
            .map(|(k, v)| Ok((k, v.build()?)))
            .collect::<Result<Vec<_>, regex::Error>>()?,
    })
}

/// Process a single predicate operation and add to field builders.
fn process_predicate_operation(
    operation: &PredicateOperation,
    case_sensitive: bool,
    except_pattern: Option<String>,
    builders: &mut FieldBuilders,
) -> Result<(), regex::Error> {
    match operation {
        PredicateOperation::Equals(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                builders,
                |builder, value, cs| {
                    builder.string_pred.add_equals(value, cs);
                },
            )?;
        }
        PredicateOperation::Contains(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                builders,
                |builder, value, cs| {
                    builder.string_pred.add_contains(value, cs);
                },
            )?;
        }
        PredicateOperation::StartsWith(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                builders,
                |builder, value, cs| {
                    builder.string_pred.add_starts_with(value, cs);
                },
            )?;
        }
        PredicateOperation::EndsWith(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                builders,
                |builder, value, cs| {
                    builder.string_pred.add_ends_with(value, cs);
                },
            )?;
        }
        PredicateOperation::Matches(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                builders,
                |builder, value, _cs| {
                    builder.string_pred.add_regex(value);
                },
            )?;
        }
        PredicateOperation::And(children) => {
            // AND predicates naturally combine by adding to the same field builders
            for child in children {
                let child_case_sensitive = child.parameters.case_sensitive.unwrap_or(case_sensitive);
                let child_except = if child.parameters.except.is_empty() {
                    except_pattern.clone()
                } else {
                    Some(child.parameters.except.clone())
                };
                process_predicate_operation(
                    &child.operation,
                    child_case_sensitive,
                    child_except,
                    builders,
                )?;
            }
        }
        PredicateOperation::Or(_children) => {
            // OR predicates are not yet optimized - this would require a different approach
            // For now, we skip OR optimization and fall back to the original implementation
            // TODO: Implement OR optimization
        }
        PredicateOperation::Not(_inner) => {
            // NOT predicates are complex to optimize - skip for now
            // TODO: Implement NOT optimization
        }
        PredicateOperation::DeepEquals(_fields) => {
            // DeepEquals is complex - skip optimization for now
            // TODO: Implement DeepEquals optimization
        }
        PredicateOperation::Exists(_fields) => {
            // Exists checks are different - skip optimization for now
            // TODO: Implement Exists optimization
        }
    }

    Ok(())
}

/// Process fields from a predicate operation and add to appropriate builders.
fn process_fields<F>(
    fields: &HashMap<String, serde_json::Value>,
    case_sensitive: bool,
    except_pattern: Option<&String>,
    builders: &mut FieldBuilders,
    mut add_to_builder: F,
) -> Result<(), regex::Error>
where
    F: FnMut(&mut FieldPredicateBuilder, String, bool),
{
    for (field_name, value) in fields {
        let value_str = match value {
            serde_json::Value::String(s) => s.clone(),
            _ => value.to_string(),
        };

        match field_name.as_str() {
            "method" => {
                if let Some(except) = except_pattern {
                    builders.method.except_pattern = Some(except.clone());
                }
                add_to_builder(&mut builders.method, value_str, case_sensitive);
            }
            "path" => {
                if let Some(except) = except_pattern {
                    builders.path.except_pattern = Some(except.clone());
                }
                add_to_builder(&mut builders.path, value_str, case_sensitive);
            }
            "body" => {
                if let Some(except) = except_pattern {
                    builders.body.except_pattern = Some(except.clone());
                }
                add_to_builder(&mut builders.body, value_str, case_sensitive);
            }
            "requestFrom" => {
                if let Some(except) = except_pattern {
                    builders.request_from.except_pattern = Some(except.clone());
                }
                add_to_builder(&mut builders.request_from, value_str, case_sensitive);
            }
            "ip" => {
                if let Some(except) = except_pattern {
                    builders.ip.except_pattern = Some(except.clone());
                }
                add_to_builder(&mut builders.ip, value_str, case_sensitive);
            }
            "query" => {
                // Query is an object with parameter names as keys
                if let Some(obj) = value.as_object() {
                    for (param_name, param_value) in obj {
                        let param_value_str = match param_value {
                            serde_json::Value::String(s) => s.clone(),
                            _ => param_value.to_string(),
                        };
                        let builder = builders.query.entry(param_name.clone()).or_default();
                        if let Some(except) = except_pattern {
                            builder.except_pattern = Some(except.clone());
                        }
                        add_to_builder(builder, param_value_str, case_sensitive);
                    }
                }
            }
            "headers" => {
                // Headers is an object with header names as keys
                if let Some(obj) = value.as_object() {
                    for (header_name, header_value) in obj {
                        let header_value_str = match header_value {
                            serde_json::Value::String(s) => s.clone(),
                            _ => header_value.to_string(),
                        };
                        // Lowercase header names for case-insensitive matching
                        let builder = builders
                            .headers
                            .entry(header_name.to_lowercase())
                            .or_default();
                        if let Some(except) = except_pattern {
                            builder.except_pattern = Some(except.clone());
                        }
                        add_to_builder(builder, header_value_str, case_sensitive);
                    }
                }
            }
            "form" => {
                // Form is an object with field names as keys
                if let Some(obj) = value.as_object() {
                    for (form_name, form_value) in obj {
                        let form_value_str = match form_value {
                            serde_json::Value::String(s) => s.clone(),
                            _ => form_value.to_string(),
                        };
                        let builder = builders.form.entry(form_name.clone()).or_default();
                        if let Some(except) = except_pattern {
                            builder.except_pattern = Some(except.clone());
                        }
                        add_to_builder(builder, form_value_str, case_sensitive);
                    }
                }
            }
            _ => {
                // Unknown field - ignore for now
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imposter::types::PredicateParameters;

    fn make_predicate(operation: PredicateOperation, case_sensitive: bool) -> Predicate {
        Predicate {
            parameters: PredicateParameters {
                case_sensitive: Some(case_sensitive),
                ..Default::default()
            },
            operation,
        }
    }

    #[test]
    fn test_optimize_simple_predicates() {
        let predicates = vec![
            make_predicate(
                PredicateOperation::StartsWith(
                    [("body".to_string(), serde_json::json!("abc"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Contains(
                    [("body".to_string(), serde_json::json!("123"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Contains(
                    [("body".to_string(), serde_json::json!("456"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
        ];

        let optimized = optimize_predicates(&predicates).unwrap();

        // Should have body predicate
        assert!(optimized.body.is_some());

        // Test matching
        let body_pred = optimized.body.unwrap();
        assert!(body_pred.matches("abc123456")); // starts with abc, contains 123 and 456
        assert!(!body_pred.matches("123456")); // doesn't start with abc
        assert!(!body_pred.matches("abc456")); // doesn't contain 123
    }

    #[test]
    fn test_optimize_regex_predicates() {
        let predicates = vec![
            make_predicate(
                PredicateOperation::Matches(
                    [("path".to_string(), serde_json::json!(r"^/my_path/\d+$"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Matches(
                    [("body".to_string(), serde_json::json!(r"busy-\d+"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
        ];

        let optimized = optimize_predicates(&predicates).unwrap();

        // Should have both path and body predicates
        assert!(optimized.path.is_some());
        assert!(optimized.body.is_some());

        // Test path matching
        let path_pred = optimized.path.unwrap();
        assert!(path_pred.matches("/my_path/123"));
        assert!(!path_pred.matches("/my_path/abc"));

        // Test body matching
        let body_pred = optimized.body.unwrap();
        assert!(body_pred.matches("busy-42"));
        assert!(!body_pred.matches("busy-abc"));
    }

    #[test]
    fn test_optimize_combined_predicates() {
        // Test combining simple operations with regexes on the same field
        let predicates = vec![
            make_predicate(
                PredicateOperation::StartsWith(
                    [("body".to_string(), serde_json::json!("abc"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Contains(
                    [("body".to_string(), serde_json::json!("123"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Matches(
                    [("body".to_string(), serde_json::json!(r"busy-\d+"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
        ];

        let optimized = optimize_predicates(&predicates).unwrap();

        assert!(optimized.body.is_some());

        let body_pred = optimized.body.unwrap();
        assert!(body_pred.matches("abc123busy-42")); // starts with abc, contains 123, matches regex
        assert!(!body_pred.matches("abc123busy-abc")); // regex doesn't match
        assert!(!body_pred.matches("abc456busy-42")); // doesn't contain 123
    }

    #[test]
    fn test_optimize_multiple_fields() {
        let mut fields = HashMap::new();
        fields.insert("path".to_string(), serde_json::json!(r"^/my_path/\d+$"));
        fields.insert("body".to_string(), serde_json::json!(r"busy-\d+"));

        let predicates = vec![make_predicate(PredicateOperation::Matches(fields), true)];

        let optimized = optimize_predicates(&predicates).unwrap();

        assert!(optimized.path.is_some());
        assert!(optimized.body.is_some());
    }

    #[test]
    fn test_end_to_end_optimization() {
        // This test demonstrates the complete optimization flow from the user's example
        // Original predicates (per-type organization):
        // - startsWith: { body: "abc" }
        // - contains: { body: "123" }
        // - contains: { body: "456" }
        // - matches: { path: '^/my_path/\d+$', body: 'busy-\d+' }

        let predicates = vec![
            make_predicate(
                PredicateOperation::StartsWith(
                    [("body".to_string(), serde_json::json!("abc"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Contains(
                    [("body".to_string(), serde_json::json!("123"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Contains(
                    [("body".to_string(), serde_json::json!("456"))]
                        .iter()
                        .cloned()
                        .collect(),
                ),
                true,
            ),
            make_predicate(
                PredicateOperation::Matches({
                    let mut fields = HashMap::new();
                    fields.insert("path".to_string(), serde_json::json!(r"^/my_path/\d+$"));
                    fields.insert("body".to_string(), serde_json::json!(r"busy-\d+"));
                    fields
                }),
                true,
            ),
        ];

        // Optimize to per-field organization
        let optimized = optimize_predicates(&predicates).unwrap();

        // Verify the structure is optimized per-field
        assert!(optimized.path.is_some(), "Path predicate should exist");
        assert!(optimized.body.is_some(), "Body predicate should exist");

        // Test matching with the optimized predicates
        let query = HashMap::new();
        let headers = HashMap::new();

        // Should match: path matches regex, body starts with "abc", contains "123" and "456", and matches busy-\d+
        assert!(optimized.matches(
            "GET",
            "/my_path/123",
            &query,
            &headers,
            Some("abc123456busy-42"),
            None,
            None,
            None,
        ));

        // Should NOT match: path doesn't match regex
        assert!(!optimized.matches(
            "GET",
            "/my_path/abc",
            &query,
            &headers,
            Some("abc123456busy-42"),
            None,
            None,
            None,
        ));

        // Should NOT match: body doesn't start with "abc"
        assert!(!optimized.matches(
            "GET",
            "/my_path/123",
            &query,
            &headers,
            Some("123456busy-42"),
            None,
            None,
            None,
        ));

        // Should NOT match: body doesn't contain "123"
        assert!(!optimized.matches(
            "GET",
            "/my_path/123",
            &query,
            &headers,
            Some("abc456busy-42"),
            None,
            None,
            None,
        ));

        // Should NOT match: body doesn't contain "456"
        assert!(!optimized.matches(
            "GET",
            "/my_path/123",
            &query,
            &headers,
            Some("abc123busy-42"),
            None,
            None,
            None,
        ));

        // Should NOT match: body doesn't match busy-\d+ regex
        assert!(!optimized.matches(
            "GET",
            "/my_path/123",
            &query,
            &headers,
            Some("abc123456busy-abc"),
            None,
            None,
            None,
        ));
    }

    #[test]
    fn test_and_optimization() {
        // Test that AND predicates are naturally optimized to the same field
        // - and:
        //   - startsWith: { body: "abc" }
        //   - contains: { body: "123" }
        // - and:
        //   - contains: { body: "456" }
        //   - matches: { path: '^/my_path/\d+$', body: 'busy-\d+' }

        let predicates = vec![
            make_predicate(
                PredicateOperation::And(vec![
                    Predicate {
                        parameters: Default::default(),
                        operation: PredicateOperation::StartsWith(
                            [("body".to_string(), serde_json::json!("abc"))]
                                .iter()
                                .cloned()
                                .collect(),
                        ),
                    },
                    Predicate {
                        parameters: Default::default(),
                        operation: PredicateOperation::Contains(
                            [("body".to_string(), serde_json::json!("123"))]
                                .iter()
                                .cloned()
                                .collect(),
                        ),
                    },
                ]),
                true,
            ),
            make_predicate(
                PredicateOperation::And(vec![
                    Predicate {
                        parameters: Default::default(),
                        operation: PredicateOperation::Contains(
                            [("body".to_string(), serde_json::json!("456"))]
                                .iter()
                                .cloned()
                                .collect(),
                        ),
                    },
                    Predicate {
                        parameters: Default::default(),
                        operation: PredicateOperation::Matches({
                            let mut fields = HashMap::new();
                            fields.insert("path".to_string(), serde_json::json!(r"^/my_path/\d+$"));
                            fields.insert("body".to_string(), serde_json::json!(r"busy-\d+"));
                            fields
                        }),
                    },
                ]),
                true,
            ),
        ];

        let optimized = optimize_predicates(&predicates).unwrap();

        // Should be optimized to the same structure as the non-AND version
        assert!(optimized.path.is_some());
        assert!(optimized.body.is_some());

        // Test matching
        let query = HashMap::new();
        let headers = HashMap::new();

        assert!(optimized.matches(
            "GET",
            "/my_path/123",
            &query,
            &headers,
            Some("abc123456busy-42"),
            None,
            None,
            None,
        ));
    }
}
