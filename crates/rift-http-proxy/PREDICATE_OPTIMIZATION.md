# Predicate Optimization: Per-Field Organization

## Overview

This optimization reorganizes predicate matching from a **per-type** organization to a **per-field** organization, significantly improving cache locality and enabling RegexSet optimizations.

## Motivation

### Before (Per-Type Organization)

Predicates were organized by matcher type, requiring iteration through all predicates:

```yaml
- startsWith:
    body: "abc"
- contains:
    body: "123"
- contains:
    body: "456"
- matches:
    path: '^/my_path/\d+$'
    body: 'busy-\d+'
```

This meant:
- Multiple passes over the same field (body scanned 4 times)
- Poor cache locality
- No opportunity for batch regex matching
- Each regex compiled and checked separately

### After (Per-Field Organization)

```yaml
body:
  startsWith: "abc"
  contains: ["123", "456"]
  matches: 'busy-\d+'
path:
  matches: '^/my_path/\d+$'
```

This enables:
- Single pass over each field
- Better cache locality
- RegexSet can check multiple regexes simultaneously
- All body operations processed together

## Implementation

### Key Components

1. **`MaybeSensitiveStr`** - String with optional case-insensitive matching
   - Caches lowercase version to avoid repeated allocations
   - Provides efficient `equals`, `starts_with`, `ends_with`, `contained_in` methods

2. **`StringPredicate`** - Optimized string matching
   ```rust
   enum StringPredicate {
       Simple {
           starts_with: Option<MaybeSensitiveStr>,
           ends_with: Option<MaybeSensitiveStr>,
           contains: Vec<MaybeSensitiveStr>,
           equals: Option<MaybeSensitiveStr>,
       },
       Regexes {
           set: RegexSet,
           require_all: bool,
       },
       Combined {
           simple: Box<StringPredicate>,
           regexes: RegexSet,
           require_all_regexes: bool,
       },
   }
   ```

3. **`OptimizedPredicates`** - Per-field organization
   - Groups all predicates by field (method, path, body, etc.)
   - Enables single-pass matching per field
   - Vec for multi-value fields (headers, query params)

4. **`predicate_optimizer`** - Conversion logic
   - Converts Mountebank predicates to optimized format
   - Identifies RegexSet opportunities
   - Handles AND operations naturally

## Optimizations

### 1. RegexSet for Multiple Regex Patterns

When multiple regex patterns match the same field, they're combined into a RegexSet:

```rust
// Instead of checking 3 separate regexes:
regex1.is_match(body)
regex2.is_match(body)
regex3.is_match(body)

// Use RegexSet to check all at once:
regex_set.matches(body).matched_all()
```

RegexSet uses a DFA that can check multiple patterns in a single pass over the input.

### 2. Cache Locality

Grouping operations by field means all operations on `body` are performed together, improving CPU cache utilization.

### 3. AND Operation Optimization

AND predicates naturally combine into the same field builders:

```yaml
- and:
  - startsWith: { body: "abc" }
  - contains: { body: "123" }
```

Becomes the same optimized structure as:

```yaml
body:
  startsWith: "abc"
  contains: "123"
```

## Performance Benefits

1. **Fewer String Allocations** - Case-insensitive patterns are lowercased once
2. **Batch Regex Matching** - RegexSet checks multiple patterns simultaneously
3. **Single Pass Per Field** - Each field value is processed only once
4. **Better Cache Locality** - All operations on a field are co-located in memory
5. **Future Optimizations** - Structure enables:
   - `memmem::Finder` for case-sensitive `contains` operations
   - Bloom filters for quick rejection of non-matching values
   - SIMD optimizations for simple string operations

## Backward Compatibility

The Mountebank protocol format remains unchanged:
- Deserialization still uses the per-type format
- Optimization happens during compilation
- External APIs are unchanged

## Testing

Comprehensive tests verify:
- End-to-end optimization from Mountebank format
- AND operation optimization
- Multiple fields with mixed operations
- RegexSet integration
- Case-sensitive and case-insensitive matching

## Limitations and Future Work

### Current Limitations

1. **OR Operations** - Not yet optimized (requires different approach)
2. **NOT Operations** - Complex to optimize, falls back to original
3. **DeepEquals** - Not optimized yet
4. **Exists** - Not optimized yet

### Future Enhancements

1. Integrate `memmem` crate for optimized case-sensitive contains
2. Add OR optimization using separate predicate sets
3. Implement NOT by inverting match results
4. Add telemetry to measure performance gains
5. Consider SIMD optimizations for simple operations

## Usage Example

```rust
use rift_http_proxy::imposter::predicate_optimizer::optimize_predicates;

// Mountebank predicates (per-type)
let predicates = vec![/* ... */];

// Optimize to per-field format
let optimized = optimize_predicates(&predicates)?;

// Match request (single pass per field)
let matches = optimized.matches(
    "GET",
    "/api/users/123",
    &query_params,
    &headers,
    Some("request body"),
    Some("127.0.0.1:12345"),
    Some("127.0.0.1"),
    None,
);
```

## Benchmarks

TODO: Add benchmarks comparing:
- Original predicate matching vs optimized
- Single regex vs RegexSet for multiple patterns
- Impact of field count on performance
- Memory usage comparison
