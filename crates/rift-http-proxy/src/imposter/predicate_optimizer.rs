//! Predicate optimizer: converts Mountebank predicates to optimized per-field format.
//!
//! This module provides conversion from the Mountebank predicate format (organized per-type)
//! to our optimized per-field format that enables better cache locality and RegexSet optimization.

use super::optimized_predicates::{MaybeSensitiveStr, OptimizedPredicates, StringPredicate};
use super::types::{Predicate, PredicateOperation};
use regex::RegexSet;
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

/// Per-field builders for constructing optimized predicates.
#[derive(Debug, Default)]
struct FieldBuilders {
    method: StringPredicateBuilder,
    path: StringPredicateBuilder,
    body: StringPredicateBuilder,
    query: HashMap<String, StringPredicateBuilder>,
    headers: HashMap<String, StringPredicateBuilder>,
    request_from: StringPredicateBuilder,
    ip: StringPredicateBuilder,
    form: HashMap<String, StringPredicateBuilder>,
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
        process_predicate_operation(&predicate.operation, case_sensitive, &mut builders)?;
    }

    // Build final optimized predicates
    Ok(OptimizedPredicates {
        method: if builders.method.starts_with.is_some()
            || builders.method.ends_with.is_some()
            || !builders.method.contains.is_empty()
            || builders.method.equals.is_some()
            || !builders.method.regexes.is_empty()
        {
            Some(builders.method.build()?)
        } else {
            None
        },
        path: if builders.path.starts_with.is_some()
            || builders.path.ends_with.is_some()
            || !builders.path.contains.is_empty()
            || builders.path.equals.is_some()
            || !builders.path.regexes.is_empty()
        {
            Some(builders.path.build()?)
        } else {
            None
        },
        body: if builders.body.starts_with.is_some()
            || builders.body.ends_with.is_some()
            || !builders.body.contains.is_empty()
            || builders.body.equals.is_some()
            || !builders.body.regexes.is_empty()
        {
            Some(builders.body.build()?)
        } else {
            None
        },
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
        request_from: if builders.request_from.starts_with.is_some()
            || builders.request_from.ends_with.is_some()
            || !builders.request_from.contains.is_empty()
            || builders.request_from.equals.is_some()
            || !builders.request_from.regexes.is_empty()
        {
            Some(builders.request_from.build()?)
        } else {
            None
        },
        ip: if builders.ip.starts_with.is_some()
            || builders.ip.ends_with.is_some()
            || !builders.ip.contains.is_empty()
            || builders.ip.equals.is_some()
            || !builders.ip.regexes.is_empty()
        {
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
    builders: &mut FieldBuilders,
) -> Result<(), regex::Error> {
    match operation {
        PredicateOperation::Equals(fields) => {
            process_fields(fields, case_sensitive, builders, |builder, value, cs| {
                builder.add_equals(value, cs);
            })?;
        }
        PredicateOperation::Contains(fields) => {
            process_fields(fields, case_sensitive, builders, |builder, value, cs| {
                builder.add_contains(value, cs);
            })?;
        }
        PredicateOperation::StartsWith(fields) => {
            process_fields(fields, case_sensitive, builders, |builder, value, cs| {
                builder.add_starts_with(value, cs);
            })?;
        }
        PredicateOperation::EndsWith(fields) => {
            process_fields(fields, case_sensitive, builders, |builder, value, cs| {
                builder.add_ends_with(value, cs);
            })?;
        }
        PredicateOperation::Matches(fields) => {
            process_fields(fields, case_sensitive, builders, |builder, value, _cs| {
                builder.add_regex(value);
            })?;
        }
        PredicateOperation::And(children) => {
            // AND predicates naturally combine by adding to the same field builders
            for child in children {
                let child_case_sensitive = child.parameters.case_sensitive.unwrap_or(case_sensitive);
                process_predicate_operation(&child.operation, child_case_sensitive, builders)?;
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
    builders: &mut FieldBuilders,
    mut add_to_builder: F,
) -> Result<(), regex::Error>
where
    F: FnMut(&mut StringPredicateBuilder, String, bool),
{
    for (field_name, value) in fields {
        let value_str = match value {
            serde_json::Value::String(s) => s.clone(),
            _ => value.to_string(),
        };

        match field_name.as_str() {
            "method" => add_to_builder(&mut builders.method, value_str, case_sensitive),
            "path" => add_to_builder(&mut builders.path, value_str, case_sensitive),
            "body" => add_to_builder(&mut builders.body, value_str, case_sensitive),
            "requestFrom" => add_to_builder(&mut builders.request_from, value_str, case_sensitive),
            "ip" => add_to_builder(&mut builders.ip, value_str, case_sensitive),
            "query" => {
                // Query is an object with parameter names as keys
                if let Some(obj) = value.as_object() {
                    for (param_name, param_value) in obj {
                        let param_value_str = match param_value {
                            serde_json::Value::String(s) => s.clone(),
                            _ => param_value.to_string(),
                        };
                        let builder = builders.query.entry(param_name.clone()).or_default();
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
