//! Predicate optimizer: converts Mountebank predicates to optimized per-field format.
//!
//! This module provides conversion from the Mountebank predicate format (organized per-type)
//! to our optimized per-field format that enables better cache locality and RegexSet optimization.

use super::optimized_predicates::{
    FieldPredicate, MaybeSensitiveStr, ObjectPredicate, OptimizedPredicates, StringPredicate,
    ValuePredicate, ValueSelector,
};
use super::types::{Predicate, PredicateOperation, PredicateSelector};
use regex::{Regex, RegexSet};
use serde_json::Value as JsonValue;
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
    /// Case-insensitive contains operations are converted to regexes.
    /// If regex compilation fails, returns a Never predicate that never matches.
    fn build(self) -> StringPredicate {
        // Separate case-sensitive and case-insensitive contains
        let mut case_sensitive_contains = Vec::new();
        let mut case_insensitive_regexes = Vec::new();

        for (pattern, case_sensitive) in self.contains {
            if case_sensitive {
                case_sensitive_contains.push(pattern);
            } else {
                // Convert case-insensitive contains to regex with (?i) flag
                // Escape special regex characters
                let escaped = regex::escape(&pattern);
                case_insensitive_regexes.push(format!("(?i){}", escaped));
            }
        }

        // Merge case-insensitive contains regexes with explicit regexes
        let mut all_regexes = self.regexes;
        all_regexes.extend(case_insensitive_regexes);

        let has_simple = self.starts_with.is_some()
            || self.ends_with.is_some()
            || !case_sensitive_contains.is_empty()
            || self.equals.is_some();
        let has_regexes = !all_regexes.is_empty();

        match (has_simple, has_regexes) {
            (false, false) => {
                // No constraints at all
                StringPredicate::empty_simple()
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

                // Only add case-sensitive contains (case-insensitive are converted to regex)
                for pattern in case_sensitive_contains {
                    pred = pred.with_contains(MaybeSensitiveStr::new(pattern, true));
                }

                if let Some((pattern, case_sensitive)) = self.equals {
                    pred = pred.with_equals(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                pred
            }
            (false, true) => {
                // Only regexes - use RegexSet
                match RegexSet::new(&all_regexes) {
                    Ok(set) => StringPredicate::Regexes {
                        set,
                        require_all: true, // All regexes must match (AND)
                    },
                    Err(e) => {
                        tracing::warn!("Failed to compile regex patterns: {}", e);
                        StringPredicate::Never
                    }
                }
            }
            (true, true) => {
                // Both simple and regexes - use Combined
                let mut simple = StringPredicate::empty_simple();

                if let Some((pattern, case_sensitive)) = self.starts_with {
                    simple =
                        simple.with_starts_with(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                if let Some((pattern, case_sensitive)) = self.ends_with {
                    simple = simple.with_ends_with(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                // Only add case-sensitive contains
                for pattern in case_sensitive_contains {
                    simple = simple.with_contains(MaybeSensitiveStr::new(pattern, true));
                }

                if let Some((pattern, case_sensitive)) = self.equals {
                    simple = simple.with_equals(MaybeSensitiveStr::new(pattern, case_sensitive));
                }

                match RegexSet::new(&all_regexes) {
                    Ok(regexes) => StringPredicate::Combined {
                        simple: Box::new(simple),
                        regexes,
                        require_all_regexes: true,
                    },
                    Err(e) => {
                        tracing::warn!("Failed to compile regex patterns: {}", e);
                        StringPredicate::Never
                    }
                }
            }
        }
    }
}

/// Builder for a FieldPredicate (ValuePredicate + except pattern + selector).
#[derive(Debug, Default)]
struct FieldPredicateBuilder {
    string_pred: StringPredicateBuilder,
    object_pred: Option<ObjectPredicate>,
    except_pattern: Option<String>,
    selector: Option<PredicateSelector>,
}

impl FieldPredicateBuilder {
    fn build(self) -> FieldPredicate {
        // Convert PredicateSelector to ValueSelector if present
        let value_selector = self.selector.map(|s| match s {
            PredicateSelector::JsonPath { selector } => ValueSelector::JsonPath(selector),
            PredicateSelector::XPath {
                selector,
                namespaces,
            } => ValueSelector::XPath {
                selector,
                namespaces,
            },
        });

        // Build the ValuePredicate (either String or Object)
        let value_pred = if let Some(obj_pred) = self.object_pred {
            ValuePredicate::Object(obj_pred)
        } else {
            ValuePredicate::String(self.string_pred.build())
        };

        match (self.except_pattern, value_selector) {
            (Some(pattern), Some(selector)) => match Regex::new(&pattern) {
                Ok(except_regex) => FieldPredicate::with_except_and_selector_value(
                    value_pred,
                    except_regex,
                    selector,
                ),
                Err(e) => {
                    tracing::warn!("Failed to compile except pattern regex: {}", e);
                    // If except pattern is invalid, create a Never predicate
                    FieldPredicate::new_value(ValuePredicate::String(StringPredicate::Never))
                }
            },
            (Some(pattern), None) => match Regex::new(&pattern) {
                Ok(except_regex) => FieldPredicate::with_except_value(value_pred, except_regex),
                Err(e) => {
                    tracing::warn!("Failed to compile except pattern regex: {}", e);
                    FieldPredicate::new_value(ValuePredicate::String(StringPredicate::Never))
                }
            },
            (None, Some(selector)) => FieldPredicate::with_selector_value(value_pred, selector),
            (None, None) => FieldPredicate::new_value(value_pred),
        }
    }

    fn is_empty(&self) -> bool {
        self.string_pred.starts_with.is_none()
            && self.string_pred.ends_with.is_none()
            && self.string_pred.contains.is_empty()
            && self.string_pred.equals.is_none()
            && self.string_pred.regexes.is_empty()
            && self.object_pred.is_none()
            && self.except_pattern.is_none()
            && self.selector.is_none()
    }
}

/// Key for grouping predicates by selector.
///
/// Predicates with the same selector can be optimized together,
/// but predicates with different selectors must be kept separate.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum SelectorKey {
    NoSelector,
    JsonPath(String),
    XPath(String), // Just use selector string for key, ignore namespaces for grouping
}

impl From<&Option<PredicateSelector>> for SelectorKey {
    fn from(selector: &Option<PredicateSelector>) -> Self {
        match selector {
            None => SelectorKey::NoSelector,
            Some(PredicateSelector::JsonPath { selector }) => {
                SelectorKey::JsonPath(selector.clone())
            }
            Some(PredicateSelector::XPath { selector, .. }) => SelectorKey::XPath(selector.clone()),
        }
    }
}

/// Per-field builders for constructing optimized predicates.
/// All fields now support grouping by selector.
#[derive(Debug, Default)]
struct FieldBuilders {
    /// Method builders grouped by selector
    method: HashMap<SelectorKey, FieldPredicateBuilder>,
    /// Path builders grouped by selector
    path: HashMap<SelectorKey, FieldPredicateBuilder>,
    /// Body builders grouped by selector
    body: HashMap<SelectorKey, FieldPredicateBuilder>,
    /// Query builders: param name -> selector -> builder
    query: HashMap<String, HashMap<SelectorKey, FieldPredicateBuilder>>,
    /// Header builders: header name -> selector -> builder
    headers: HashMap<String, HashMap<SelectorKey, FieldPredicateBuilder>>,
    /// RequestFrom builders grouped by selector
    request_from: HashMap<SelectorKey, FieldPredicateBuilder>,
    /// IP builders grouped by selector
    ip: HashMap<SelectorKey, FieldPredicateBuilder>,
    /// Form builders: field name -> selector -> builder
    form: HashMap<String, HashMap<SelectorKey, FieldPredicateBuilder>>,
}

/// Convert Mountebank predicates to optimized per-field format.
///
/// This function analyzes all predicates and groups operations by field,
/// enabling optimizations like RegexSet for multiple regex patterns on the same field.
pub fn optimize_predicates(predicates: &[Predicate]) -> OptimizedPredicates {
    let mut builders = FieldBuilders::default();

    // Process each predicate and add to appropriate field builders
    for predicate in predicates {
        let case_sensitive = predicate.parameters.case_sensitive.unwrap_or(false);
        let except_pattern = if predicate.parameters.except.is_empty() {
            None
        } else {
            Some(predicate.parameters.except.clone())
        };
        let selector = predicate.parameters.selector.clone();

        process_predicate_operation(
            &predicate.operation,
            case_sensitive,
            except_pattern,
            selector,
            &mut builders,
        );
    }

    // Build final optimized predicates
    OptimizedPredicates {
        // Build all method predicates (one per unique selector)
        method: builders
            .method
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(_, v)| v.build())
            .collect(),
        // Build all path predicates (one per unique selector)
        path: builders
            .path
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(_, v)| v.build())
            .collect(),
        // Build all body predicates (one per unique selector)
        body: builders
            .body
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(_, v)| v.build())
            .collect(),
        // Build query predicates: param name -> Vec<FieldPredicate>
        query: builders
            .query
            .into_iter()
            .map(|(param_name, selector_map)| {
                let preds: Vec<FieldPredicate> = selector_map
                    .into_iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(_, v)| v.build())
                    .collect();
                (param_name, preds)
            })
            .filter(|(_, v)| !v.is_empty())
            .collect(),
        // Build header predicates: header name -> Vec<FieldPredicate>
        headers: builders
            .headers
            .into_iter()
            .map(|(header_name, selector_map)| {
                let preds: Vec<FieldPredicate> = selector_map
                    .into_iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(_, v)| v.build())
                    .collect();
                (header_name, preds)
            })
            .filter(|(_, v)| !v.is_empty())
            .collect(),
        // Build all request_from predicates (one per unique selector)
        request_from: builders
            .request_from
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(_, v)| v.build())
            .collect(),
        // Build all ip predicates (one per unique selector)
        ip: builders
            .ip
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(_, v)| v.build())
            .collect(),
        // Build form predicates: field name -> Vec<FieldPredicate>
        form: builders
            .form
            .into_iter()
            .map(|(field_name, selector_map)| {
                let preds: Vec<FieldPredicate> = selector_map
                    .into_iter()
                    .filter(|(_, v)| !v.is_empty())
                    .map(|(_, v)| v.build())
                    .collect();
                (field_name, preds)
            })
            .filter(|(_, v)| !v.is_empty())
            .collect(),
    }
}

/// Process a single predicate operation and add to field builders.
fn process_predicate_operation(
    operation: &PredicateOperation,
    case_sensitive: bool,
    except_pattern: Option<String>,
    selector: Option<PredicateSelector>,
    builders: &mut FieldBuilders,
) {
    match operation {
        PredicateOperation::Equals(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                selector.as_ref(),
                builders,
                PredicateOperationType::Equals,
                |builder, value, cs| {
                    builder.string_pred.add_equals(value, cs);
                },
            );
        }
        PredicateOperation::Contains(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                selector.as_ref(),
                builders,
                PredicateOperationType::Contains,
                |builder, value, cs| {
                    builder.string_pred.add_contains(value, cs);
                },
            );
        }
        PredicateOperation::StartsWith(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                selector.as_ref(),
                builders,
                PredicateOperationType::StartsWith,
                |builder, value, cs| {
                    builder.string_pred.add_starts_with(value, cs);
                },
            );
        }
        PredicateOperation::EndsWith(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                selector.as_ref(),
                builders,
                PredicateOperationType::EndsWith,
                |builder, value, cs| {
                    builder.string_pred.add_ends_with(value, cs);
                },
            );
        }
        PredicateOperation::Matches(fields) => {
            process_fields(
                fields,
                case_sensitive,
                except_pattern.as_ref(),
                selector.as_ref(),
                builders,
                PredicateOperationType::Matches,
                |builder, value, _cs| {
                    builder.string_pred.add_regex(value);
                },
            );
        }
        PredicateOperation::And(children) => {
            // AND predicates naturally combine by adding to the same field builders
            for child in children {
                let child_case_sensitive =
                    child.parameters.case_sensitive.unwrap_or(case_sensitive);
                let child_except = if child.parameters.except.is_empty() {
                    except_pattern.clone()
                } else {
                    Some(child.parameters.except.clone())
                };
                let child_selector = child
                    .parameters
                    .selector
                    .clone()
                    .or_else(|| selector.clone());
                process_predicate_operation(
                    &child.operation,
                    child_case_sensitive,
                    child_except,
                    child_selector,
                    builders,
                );
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
}

/// Process fields from a predicate operation and add to appropriate builders.
/// Helper to add an object predicate to a builder.
fn add_object_to_builder(
    builder: &mut FieldPredicateBuilder,
    value: &JsonValue,
    operation_type: PredicateOperationType,
) {
    let obj_pred = match operation_type {
        PredicateOperationType::Equals => ObjectPredicate::Equals(value.clone()),
        PredicateOperationType::DeepEquals => ObjectPredicate::DeepEquals(value.clone()),
        PredicateOperationType::Contains => ObjectPredicate::Contains(value.clone()),
        PredicateOperationType::Matches => {
            // For matches with object, each value should be a regex pattern
            if let JsonValue::Object(obj) = value {
                let mut regexes = HashMap::new();
                for (key, val) in obj {
                    if let JsonValue::String(pattern) = val {
                        match Regex::new(pattern) {
                            Ok(regex) => {
                                regexes.insert(key.clone(), regex);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to compile regex pattern for key '{}': {}",
                                    key,
                                    e
                                );
                                // Skip this regex, continue with others
                            }
                        }
                    }
                }
                ObjectPredicate::Matches(regexes)
            } else {
                return; // Skip non-object matches
            }
        }
        _ => return, // Other operations don't support objects
    };
    builder.object_pred = Some(obj_pred);
}

#[derive(Debug, Clone, Copy)]
enum PredicateOperationType {
    Equals,
    DeepEquals,
    Contains,
    StartsWith,
    EndsWith,
    Matches,
}

fn process_fields<F>(
    fields: &HashMap<String, serde_json::Value>,
    case_sensitive: bool,
    except_pattern: Option<&String>,
    selector: Option<&PredicateSelector>,
    builders: &mut FieldBuilders,
    operation_type: PredicateOperationType,
    mut add_string_to_builder: F,
) where
    F: FnMut(&mut FieldPredicateBuilder, String, bool),
{
    for (field_name, value) in fields {
        // Check if this is an object value (for body, query, headers, form)
        let is_object_value = value.is_object() || value.is_array();

        match field_name.as_str() {
            "method" => {
                let selector_key = SelectorKey::from(&selector.cloned());
                let builder = builders.method.entry(selector_key).or_default();
                if let Some(except) = except_pattern {
                    builder.except_pattern = Some(except.clone());
                }
                if let Some(sel) = selector {
                    builder.selector = Some(sel.clone());
                }
                if is_object_value {
                    add_object_to_builder(builder, value, operation_type);
                } else {
                    let value_str = value.as_str().unwrap_or("").to_string();
                    add_string_to_builder(builder, value_str, case_sensitive);
                }
            }
            "path" => {
                let selector_key = SelectorKey::from(&selector.cloned());
                let builder = builders.path.entry(selector_key).or_default();
                if let Some(except) = except_pattern {
                    builder.except_pattern = Some(except.clone());
                }
                if let Some(sel) = selector {
                    builder.selector = Some(sel.clone());
                }
                if is_object_value {
                    add_object_to_builder(builder, value, operation_type);
                } else {
                    let value_str = value.as_str().unwrap_or("").to_string();
                    add_string_to_builder(builder, value_str, case_sensitive);
                }
            }
            "body" => {
                // Group body predicates by selector
                let selector_key = SelectorKey::from(&selector.cloned());
                let builder = builders.body.entry(selector_key).or_default();
                if let Some(except) = except_pattern {
                    builder.except_pattern = Some(except.clone());
                }
                if let Some(sel) = selector {
                    builder.selector = Some(sel.clone());
                }
                if is_object_value {
                    // Object matching (JSON body)
                    add_object_to_builder(builder, value, operation_type);
                } else {
                    // String matching
                    let value_str = value.as_str().unwrap_or("").to_string();
                    add_string_to_builder(builder, value_str, case_sensitive);
                }
            }
            "requestFrom" => {
                let selector_key = SelectorKey::from(&selector.cloned());
                let builder = builders.request_from.entry(selector_key).or_default();
                if let Some(except) = except_pattern {
                    builder.except_pattern = Some(except.clone());
                }
                if let Some(sel) = selector {
                    builder.selector = Some(sel.clone());
                }
                if is_object_value {
                    add_object_to_builder(builder, value, operation_type);
                } else {
                    let value_str = value.as_str().unwrap_or("").to_string();
                    add_string_to_builder(builder, value_str, case_sensitive);
                }
            }
            "ip" => {
                let selector_key = SelectorKey::from(&selector.cloned());
                let builder = builders.ip.entry(selector_key).or_default();
                if let Some(except) = except_pattern {
                    builder.except_pattern = Some(except.clone());
                }
                if let Some(sel) = selector {
                    builder.selector = Some(sel.clone());
                }
                if is_object_value {
                    add_object_to_builder(builder, value, operation_type);
                } else {
                    let value_str = value.as_str().unwrap_or("").to_string();
                    add_string_to_builder(builder, value_str, case_sensitive);
                }
            }
            "query" => {
                // Query is an object with parameter names as keys
                if let Some(obj) = value.as_object() {
                    for (param_name, param_value) in obj {
                        let selector_key = SelectorKey::from(&selector.cloned());
                        let builder = builders
                            .query
                            .entry(param_name.clone())
                            .or_default()
                            .entry(selector_key)
                            .or_default();
                        if let Some(except) = except_pattern {
                            builder.except_pattern = Some(except.clone());
                        }
                        if let Some(sel) = selector {
                            builder.selector = Some(sel.clone());
                        }
                        if param_value.is_object() || param_value.is_array() {
                            add_object_to_builder(builder, param_value, operation_type);
                        } else {
                            let param_value_str = param_value.as_str().unwrap_or("").to_string();
                            add_string_to_builder(builder, param_value_str, case_sensitive);
                        }
                    }
                }
            }
            "headers" => {
                // Headers is an object with header names as keys
                if let Some(obj) = value.as_object() {
                    for (header_name, header_value) in obj {
                        // Lowercase header names for case-insensitive matching
                        let selector_key = SelectorKey::from(&selector.cloned());
                        let builder = builders
                            .headers
                            .entry(header_name.to_lowercase())
                            .or_default()
                            .entry(selector_key)
                            .or_default();
                        if let Some(except) = except_pattern {
                            builder.except_pattern = Some(except.clone());
                        }
                        if let Some(sel) = selector {
                            builder.selector = Some(sel.clone());
                        }
                        if header_value.is_object() || header_value.is_array() {
                            add_object_to_builder(builder, header_value, operation_type);
                        } else {
                            let header_value_str = header_value.as_str().unwrap_or("").to_string();
                            add_string_to_builder(builder, header_value_str, case_sensitive);
                        }
                    }
                }
            }
            "form" => {
                // Form is an object with field names as keys
                if let Some(obj) = value.as_object() {
                    for (form_name, form_value) in obj {
                        let selector_key = SelectorKey::from(&selector.cloned());
                        let builder = builders
                            .form
                            .entry(form_name.clone())
                            .or_default()
                            .entry(selector_key)
                            .or_default();
                        if let Some(except) = except_pattern {
                            builder.except_pattern = Some(except.clone());
                        }
                        if let Some(sel) = selector {
                            builder.selector = Some(sel.clone());
                        }
                        if form_value.is_object() || form_value.is_array() {
                            add_object_to_builder(builder, form_value, operation_type);
                        } else {
                            let form_value_str = form_value.as_str().unwrap_or("").to_string();
                            add_string_to_builder(builder, form_value_str, case_sensitive);
                        }
                    }
                }
            }
            _ => {
                // Unknown field - ignore for now
            }
        }
    }
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

        let optimized = optimize_predicates(&predicates);

        // Should have body predicate
        assert!(!optimized.body.is_empty());

        // Test matching
        let body_pred = &optimized.body[0];
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

        let optimized = optimize_predicates(&predicates);

        // Should have both path and body predicates
        assert!(!optimized.path.is_empty());
        assert!(!optimized.body.is_empty());

        // Test path matching
        let path_pred = &optimized.path[0];
        assert!(path_pred.matches("/my_path/123"));
        assert!(!path_pred.matches("/my_path/abc"));

        // Test body matching
        let body_pred = &optimized.body[0];
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

        let optimized = optimize_predicates(&predicates);

        assert!(!optimized.body.is_empty());

        let body_pred = &optimized.body[0];
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

        let optimized = optimize_predicates(&predicates);

        assert!(!optimized.path.is_empty());
        assert!(!optimized.body.is_empty());
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
        let optimized = optimize_predicates(&predicates);

        // Verify the structure is optimized per-field
        assert!(!optimized.path.is_empty(), "Path predicate should exist");
        assert!(!optimized.body.is_empty(), "Body predicate should exist");

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

        let optimized = optimize_predicates(&predicates);

        // Should be optimized to the same structure as the non-AND version
        assert!(!optimized.path.is_empty());
        assert!(!optimized.body.is_empty());

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

    #[test]
    fn test_object_matching() {
        // Test that object values are matched correctly
        // equals: { body: { xyz: "Hi" } }
        // Should match: {"a": "ignored", "xyz": "Hi"}

        let mut fields = HashMap::new();
        fields.insert(
            "body".to_string(),
            serde_json::json!({
                "xyz": "Hi"
            }),
        );

        let predicates = vec![make_predicate(PredicateOperation::Equals(fields), true)];

        let optimized = optimize_predicates(&predicates);

        assert!(!optimized.body.is_empty(), "Body predicate should exist");

        // Test matching
        let query = HashMap::new();
        let headers = HashMap::new();

        // Should match: body contains xyz: "Hi" (subset match)
        assert!(optimized.matches(
            "GET",
            "/any/path",
            &query,
            &headers,
            Some(r#"{"a": "ignored", "xyz": "Hi"}"#),
            None,
            None,
            None,
        ));

        // Should match: exact match
        assert!(optimized.matches(
            "GET",
            "/any/path",
            &query,
            &headers,
            Some(r#"{"xyz": "Hi"}"#),
            None,
            None,
            None,
        ));

        // Should NOT match: xyz value is different
        assert!(!optimized.matches(
            "GET",
            "/any/path",
            &query,
            &headers,
            Some(r#"{"a": "ignored", "xyz": "Bye"}"#),
            None,
            None,
            None,
        ));

        // Should NOT match: missing xyz field
        assert!(!optimized.matches(
            "GET",
            "/any/path",
            &query,
            &headers,
            Some(r#"{"a": "ignored"}"#),
            None,
            None,
            None,
        ));

        // Should NOT match: not valid JSON
        assert!(!optimized.matches(
            "GET",
            "/any/path",
            &query,
            &headers,
            Some("not json"),
            None,
            None,
            None,
        ));
    }
}
