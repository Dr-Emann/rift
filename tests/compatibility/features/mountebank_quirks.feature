Feature: Mountebank Quirks and Edge Cases
  Tests documenting unusual behaviors and quirks in Mountebank predicates and configuration
  Based on findings from https://github.com/EtaCassiopeia/rift/discussions/15

  These tests verify whether Rift maintains compatibility with Mountebank's quirks or
  implements more intuitive/strict behavior.

  Background:
    Given both Mountebank and Rift services are running
    And all imposters are cleared

  # ==========================================================================
  # Predicate Type Coercion Quirks
  # ==========================================================================

  Scenario: DeepEquals coerces numeric values to strings
    # Mountebank quirk: deepEquals coerces all scalar values to strings
    # So numeric 123 matches string "123" and vice versa
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "query": {
              "id": "123",
              "count": "5"
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "type coercion matched"}}]
      }
      """
    When I send GET request to "/?id=123&count=5" on imposter 4545
    Then both services should return status 200
    And both responses should have body "type coercion matched"

  Scenario: DeepEquals coerces boolean values to strings
    # Tests that booleans are coerced to strings
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "query": {
              "active": "true",
              "enabled": "false"
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "boolean coercion"}}]
      }
      """
    When I send GET request to "/?active=true&enabled=false" on imposter 4545
    Then both services should return status 200
    And both responses should have body "boolean coercion"

  Scenario: DeepEquals coerces null to string "null"
    # Tests that null values are coerced to the string "null"
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "body": {
              "value": "null"
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "null coercion"}}]
      }
      """
    When I send POST request with JSON body '{"value": null}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "null coercion"

  # ==========================================================================
  # Array Sorting Quirks
  # ==========================================================================

  Scenario: DeepEquals sorts arrays before comparison
    # Mountebank quirk: Arrays are sorted during comparison
    # So ["2", "1"] matches ["1", "2"]
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "body": {
              "items": ["1", "2", "3"]
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "array sorted and matched"}}]
      }
      """
    When I send POST request with JSON body '{"items": ["3", "1", "2"]}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "array sorted and matched"

  Scenario: DeepEquals sorts nested arrays
    # Tests array sorting with nested structures
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "body": {
              "data": {
                "tags": ["alpha", "beta", "gamma"]
              }
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "nested array sorted"}}]
      }
      """
    When I send POST request with JSON body '{"data": {"tags": ["gamma", "alpha", "beta"]}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "nested array sorted"

  Scenario: DeepEquals with query parameter array values
    # Query parameters can have multiple values for same key
    # Tests if arrays are sorted in query parameter matching
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "query": {
              "filter": ["a", "b", "c"]
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "query array sorted"}}]
      }
      """
    When I send GET request to "/?filter=c&filter=a&filter=b" on imposter 4545
    Then both services should return status 200
    And both responses should have body "query array sorted"

  # ==========================================================================
  # Nested Object Matching
  # ==========================================================================

  Scenario: Predicates work with nested objects recursively
    # Tests that predicates like matches, equals, contains work on nested objects
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "contains": {
            "body": {
              "user": {
                "email": "@example.com"
              }
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "nested contains matched"}}]
      }
      """
    When I send POST request with JSON body '{"user": {"name": "john", "email": "john@example.com"}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "nested contains matched"

  Scenario: StartsWith predicate on nested object fields
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "startsWith": {
            "body": {
              "config": {
                "url": "https://"
              }
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "nested startsWith matched"}}]
      }
      """
    When I send POST request with JSON body '{"config": {"url": "https://example.com", "timeout": 30}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "nested startsWith matched"

  Scenario: Matches predicate on deeply nested fields
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "matches": {
            "body": {
              "data": {
                "attributes": {
                  "id": "^[0-9]+$"
                }
              }
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "deep nested regex matched"}}]
      }
      """
    When I send POST request with JSON body '{"data": {"attributes": {"id": "12345", "name": "test"}}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "deep nested regex matched"

  # ==========================================================================
  # First Predicate Wins
  # ==========================================================================

  Scenario: Only first recognized predicate type is processed
    # Mountebank quirk: Only the first predicate type in an object is processed
    # Subsequent predicates are ignored even if valid
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "not found"},
        "stubs": [{
          "predicates": [{
            "equals": {"path": "/test"},
            "contains": {"path": "other"}
          }],
          "responses": [{"is": {"statusCode": 200, "body": "first predicate wins"}}]
        }]
      }
      """
    When I send GET request to "/test" on imposter 4545
    Then both services should return status 200
    And both responses should have body "first predicate wins"

  Scenario: First predicate processes even when second would fail
    # The second predicate (contains) would fail but is ignored
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "equals": {"path": "/api"},
          "matches": {"path": "^/different$"}
        }],
        "responses": [{"is": {"statusCode": 200, "body": "only equals processed"}}]
      }
      """
    When I send GET request to "/api" on imposter 4545
    Then both services should return status 200
    And both responses should have body "only equals processed"

  # ==========================================================================
  # Empty String Handling with Exists Predicate
  # ==========================================================================

  Scenario: Exists predicate treats empty strings as non-existent
    # Mountebank quirk: exists predicate considers empty string as not existing
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 200, "body": "exists matched"},
        "stubs": [{
          "predicates": [{
            "exists": {
              "query": {
                "param": true
              }
            }
          }],
          "responses": [{"is": {"statusCode": 404, "body": "should not match empty"}}]
        }]
      }
      """
    When I send GET request to "/?param=" on imposter 4545
    Then both services should return status 200
    And both responses should have body "exists matched"

  Scenario: Exists predicate with false matches empty strings
    # Tests that exists: false should match when field is empty string
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "exists": {
            "query": {
              "optional": false
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "field absent or empty"}}]
      }
      """
    When I send GET request to "/?other=value" on imposter 4545
    Then both services should return status 200
    And both responses should have body "field absent or empty"

  Scenario: Empty string in request body treated as non-existent by exists
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "default"},
        "stubs": [{
          "predicates": [{
            "exists": {
              "body": {
                "field": true
              }
            }
          }],
          "responses": [{"is": {"statusCode": 200, "body": "field exists"}}]
        }]
      }
      """
    When I send POST request with JSON body '{"field": ""}' on imposter 4545
    Then both services should return status 404

  # ==========================================================================
  # Modifier Inheritance with AND/OR Operators
  # ==========================================================================

  Scenario: caseSensitive does not cascade to child predicates in AND
    # Mountebank quirk: modifiers like caseSensitive don't inherit through and/or
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "not found"},
        "stubs": [{
          "predicates": [{
            "and": [
              {"equals": {"path": "/API"}},
              {"equals": {"method": "GET"}}
            ],
            "caseSensitive": false
          }],
          "responses": [{"is": {"statusCode": 200, "body": "matched"}}]
        }]
      }
      """
    When I send GET request to "/api" on imposter 4545
    Then both services should return status 404

  Scenario: except modifier does not cascade through OR predicates
    # except should not apply to child predicates in or/and
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "or": [
            {"equals": {"path": "users"}},
            {"equals": {"path": "posts"}}
          ],
          "except": "^/api/"
        }],
        "responses": [{"is": {"statusCode": 200, "body": "no except cascade"}}]
      }
      """
    When I send GET request to "/api/users" on imposter 4545
    Then both services should return status 200
    And both responses should have body "no except cascade"

  Scenario: jsonpath modifier does not cascade to AND children
    # jsonpath extraction should not apply to child predicates
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "not found"},
        "stubs": [{
          "predicates": [{
            "and": [
              {"equals": {"body": "john"}},
              {"exists": {"body": "$.user"}}
            ],
            "jsonpath": {"selector": "$.user.name"}
          }],
          "responses": [{"is": {"statusCode": 200, "body": "matched"}}]
        }]
      }
      """
    When I send POST request with JSON body '{"user": {"name": "john"}}' on imposter 4545
    Then both services should return status 404

  # ==========================================================================
  # JSONPath vs XPath Precedence
  # ==========================================================================

  Scenario: JSONPath takes complete precedence over XPath when both specified
    # Mountebank quirk: When both jsonpath and xpath are set, JSON path wins
    # and XPath becomes inert
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "equals": {"body": "john"},
          "jsonpath": {"selector": "$.user.name"},
          "xpath": {"selector": "//user/name"}
        }],
        "responses": [{"is": {"statusCode": 200, "body": "jsonpath wins"}}]
      }
      """
    When I send POST request with JSON body '{"user": {"name": "john"}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "jsonpath wins"

  Scenario: XPath ignored when JSONPath present even with XML body
    # Even with XML content, if jsonpath is present it takes precedence
    # This may cause matching to fail since JSON parsing fails on XML
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "not found"},
        "stubs": [{
          "predicates": [{
            "equals": {"body": "john"},
            "jsonpath": {"selector": "$.user.name"},
            "xpath": {"selector": "//user/name"}
          }],
          "responses": [{"is": {"statusCode": 200, "body": "should not match"}}]
        }]
      }
      """
    When I send POST request with body "<root><user><name>john</name></user></root>" on imposter 4545
    Then both services should return status 404

  # ==========================================================================
  # Exception Application Order
  # ==========================================================================

  Scenario: Except regex applies before path extraction
    # Mountebank quirk: except filter is applied before jsonpath/xpath extraction
    # This can cause unintended matches
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "equals": {"body": "john"},
          "except": "\\{.*\\}",
          "jsonpath": {"selector": "$.user.name"}
        }],
        "responses": [{"is": {"statusCode": 200, "body": "except before extraction"}}]
      }
      """
    When I send POST request with JSON body '{"user": {"name": "john"}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "except before extraction"

  Scenario: Except strips content before JSONPath can extract
    # If except removes the JSON structure, jsonpath extraction fails
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "not found"},
        "stubs": [{
          "predicates": [{
            "equals": {"body": "john"},
            "except": ".*user.*",
            "jsonpath": {"selector": "$.user.name"}
          }],
          "responses": [{"is": {"statusCode": 200, "body": "should not match"}}]
        }]
      }
      """
    When I send POST request with JSON body '{"user": {"name": "john"}}' on imposter 4545
    Then both services should return status 404

  Scenario: Except with XPath extraction ordering
    # Tests except application order with XPath
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "equals": {"body": "john"},
          "except": "<[^>]+>",
          "xpath": {"selector": "//user/name"}
        }],
        "responses": [{"is": {"statusCode": 200, "body": "except before xpath"}}]
      }
      """
    When I send POST request with body "<root><user><name>john</name></user></root>" on imposter 4545
    Then both services should return status 200
    And both responses should have body "except before xpath"

  # ==========================================================================
  # Combined Quirks - Complex Scenarios
  # ==========================================================================

  Scenario: Type coercion with array sorting in nested objects
    # Combines multiple quirks: type coercion + array sorting + nested objects
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "deepEquals": {
            "body": {
              "config": {
                "ports": ["8080", "3000", "5000"],
                "timeout": "30"
              }
            }
          }
        }],
        "responses": [{"is": {"statusCode": 200, "body": "combined quirks matched"}}]
      }
      """
    When I send POST request with JSON body '{"config": {"ports": ["5000", "8080", "3000"], "timeout": 30}}' on imposter 4545
    Then both services should return status 200
    And both responses should have body "combined quirks matched"

  Scenario: Empty string with nested object and exists predicate
    # Combines empty string handling with nested object matching
    Given an imposter on port 4545 with:
      """
      {
        "port": 4545,
        "protocol": "http",
        "defaultResponse": {"statusCode": 404, "body": "not found"},
        "stubs": [{
          "predicates": [{
            "exists": {
              "body": {
                "user": {
                  "name": true
                }
              }
            }
          }],
          "responses": [{"is": {"statusCode": 200, "body": "nested exists"}}]
        }]
      }
      """
    When I send POST request with JSON body '{"user": {"name": ""}}' on imposter 4545
    Then both services should return status 404

  Scenario: First predicate wins with modifiers
    # Tests that modifiers on ignored predicates have no effect
    Given an imposter on port 4545 with stub:
      """
      {
        "predicates": [{
          "equals": {"path": "/TEST"},
          "contains": {"path": "different"},
          "caseSensitive": false
        }],
        "responses": [{"is": {"statusCode": 200, "body": "first predicate with modifier"}}]
      }
      """
    When I send GET request to "/test" on imposter 4545
    Then both services should return status 200
    And both responses should have body "first predicate with modifier"
