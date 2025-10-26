#!/bin/bash
# Setup imposters with realistic configurations for benchmarking
# This script creates identical imposter configurations on both Mountebank and Rift

set -e

MB_URL="${MB_URL:-http://localhost:2525}"
RIFT_URL="${RIFT_URL:-http://localhost:3525}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Clear all imposters
clear_imposters() {
    local url=$1
    local name=$2
    log_info "Clearing imposters on $name..."
    curl -s -X DELETE "$url/imposters" > /dev/null
}

# Create an imposter with the given configuration
create_imposter() {
    local url=$1
    local name=$2
    local config=$3

    response=$(curl -s -w "\n%{http_code}" -X POST "$url/imposters" \
        -H "Content-Type: application/json" \
        -d "$config")

    http_code=$(echo "$response" | tail -n1)

    if [ "$http_code" != "201" ]; then
        log_error "Failed to create imposter on $name (HTTP $http_code)"
        echo "$response" | head -n -1
        return 1
    fi
}

# Generate stubs for a REST API simulation
# This simulates a typical microservice with various endpoints
generate_api_stubs() {
    local num_resources=$1
    local stubs_per_resource=$2
    local stubs=""

    for i in $(seq 1 $num_resources); do
        resource="resource${i}"

        # List endpoint
        stubs="$stubs{
            \"predicates\": [{\"equals\": {\"method\": \"GET\", \"path\": \"/api/v1/${resource}\"}}],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"items\\\": [{\\\"id\\\": 1, \\\"name\\\": \\\"${resource}_1\\\"}, {\\\"id\\\": 2, \\\"name\\\": \\\"${resource}_2\\\"}], \\\"total\\\": 2}\"
            }}]
        },"

        # Individual resource endpoints
        for j in $(seq 1 $stubs_per_resource); do
            # GET by ID
            stubs="$stubs{
                \"predicates\": [{\"equals\": {\"method\": \"GET\", \"path\": \"/api/v1/${resource}/${j}\"}}],
                \"responses\": [{\"is\": {
                    \"statusCode\": 200,
                    \"headers\": {\"Content-Type\": \"application/json\"},
                    \"body\": \"{\\\"id\\\": ${j}, \\\"name\\\": \\\"${resource}_${j}\\\", \\\"data\\\": {\\\"field1\\\": \\\"value1\\\", \\\"field2\\\": \\\"value2\\\"}}\"
                }}]
            },"

            # POST create
            stubs="$stubs{
                \"predicates\": [{
                    \"equals\": {\"method\": \"POST\", \"path\": \"/api/v1/${resource}\"},
                    \"contains\": {\"body\": \"name\"}
                }],
                \"responses\": [{\"is\": {
                    \"statusCode\": 201,
                    \"headers\": {\"Content-Type\": \"application/json\"},
                    \"body\": \"{\\\"id\\\": ${j}, \\\"created\\\": true}\"
                }}]
            },"

            # PUT update
            stubs="$stubs{
                \"predicates\": [{\"equals\": {\"method\": \"PUT\", \"path\": \"/api/v1/${resource}/${j}\"}}],
                \"responses\": [{\"is\": {
                    \"statusCode\": 200,
                    \"headers\": {\"Content-Type\": \"application/json\"},
                    \"body\": \"{\\\"id\\\": ${j}, \\\"updated\\\": true}\"
                }}]
            },"

            # DELETE
            stubs="$stubs{
                \"predicates\": [{\"equals\": {\"method\": \"DELETE\", \"path\": \"/api/v1/${resource}/${j}\"}}],
                \"responses\": [{\"is\": {
                    \"statusCode\": 204
                }}]
            },"
        done

        # Search endpoint with query params
        stubs="$stubs{
            \"predicates\": [{
                \"equals\": {\"method\": \"GET\", \"path\": \"/api/v1/${resource}/search\"},
                \"exists\": {\"query\": {\"q\": true}}
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"results\\\": [], \\\"query\\\": \\\"search\\\"}\"
            }}]
        },"
    done

    # Remove trailing comma
    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with regex matching (more CPU intensive)
generate_regex_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{\"matches\": {\"path\": \"/regex/pattern${i}/[a-zA-Z0-9]+\"}}],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"body\": \"regex match ${i}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with complex predicates (AND/OR combinations)
generate_complex_predicate_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{
                \"and\": [
                    {\"equals\": {\"method\": \"POST\"}},
                    {\"startsWith\": {\"path\": \"/complex/${i}/\"}},
                    {\"or\": [
                        {\"contains\": {\"headers\": {\"X-Request-Type\": \"json\"}}},
                        {\"contains\": {\"headers\": {\"Content-Type\": \"application/json\"}}}
                    ]}
                ]
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"complex\\\": ${i}, \\\"matched\\\": true}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with behaviors (wait, decorate)
generate_behavior_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        # Stub with wait behavior
        stubs="$stubs{
            \"predicates\": [{\"equals\": {\"path\": \"/delayed/${i}\"}}],
            \"responses\": [{
                \"is\": {\"statusCode\": 200, \"body\": \"delayed response ${i}\"},
                \"_behaviors\": {\"wait\": 10}
            }]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with JSON body matching
generate_json_body_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        # Equals JSON body
        stubs="$stubs{
            \"predicates\": [{
                \"equals\": {
                    \"method\": \"POST\",
                    \"path\": \"/json/equals/${i}\",
                    \"body\": {\"id\": ${i}, \"type\": \"request\"}
                }
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"matched\\\": \\\"equals\\\", \\\"id\\\": ${i}}\"
            }}]
        },"

        # Contains JSON body
        stubs="$stubs{
            \"predicates\": [{
                \"contains\": {
                    \"method\": \"POST\",
                    \"path\": \"/json/contains/${i}\",
                    \"body\": \"\\\"name\\\":\\\"item${i}\\\"\"
                }
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"matched\\\": \\\"contains\\\", \\\"id\\\": ${i}}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with JSONPath predicates
generate_jsonpath_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{
                \"equals\": {\"method\": \"POST\", \"path\": \"/jsonpath/${i}\"},
                \"jsonpath\": {
                    \"selector\": \"\$.user.id\",
                    \"equals\": ${i}
                }
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"jsonpath_matched\\\": true, \\\"user_id\\\": ${i}}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with XPath predicates (for XML requests)
generate_xpath_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{
                \"equals\": {\"method\": \"POST\", \"path\": \"/xpath/${i}\"},
                \"xpath\": {
                    \"selector\": \"//item[@id='${i}']\",
                    \"exists\": true
                }
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/xml\"},
                \"body\": \"<response><matched>true</matched><id>${i}</id></response>\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with template responses (injection)
generate_template_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{\"equals\": {\"path\": \"/template/${i}\"}}],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {
                    \"Content-Type\": \"application/json\",
                    \"X-Request-Path\": \"\${request.path}\",
                    \"X-Request-Method\": \"\${request.method}\"
                },
                \"body\": \"{\\\"template\\\": ${i}, \\\"path\\\": \\\"\${request.path}\\\", \\\"query\\\": \\\"\${request.query}\\\"}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with header-based routing
generate_header_routing_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{
                \"equals\": {
                    \"path\": \"/headers/route\",
                    \"headers\": {\"X-Route-Id\": \"route-${i}\"}
                }
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"routed_to\\\": ${i}}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with query parameter matching
generate_query_param_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{
                \"equals\": {
                    \"path\": \"/query/search\",
                    \"query\": {\"page\": \"${i}\", \"size\": \"10\"}
                }
            }],
            \"responses\": [{\"is\": {
                \"statusCode\": 200,
                \"headers\": {\"Content-Type\": \"application/json\"},
                \"body\": \"{\\\"page\\\": ${i}, \\\"size\\\": 10, \\\"results\\\": []}\"
            }}]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Generate stubs with decorate behavior (response modification)
generate_decorate_stubs() {
    local count=$1
    local stubs=""

    for i in $(seq 1 $count); do
        stubs="$stubs{
            \"predicates\": [{\"equals\": {\"path\": \"/decorate/${i}\"}}],
            \"responses\": [{
                \"is\": {
                    \"statusCode\": 200,
                    \"headers\": {\"Content-Type\": \"application/json\"},
                    \"body\": \"{\\\"original\\\": ${i}}\"
                },
                \"_behaviors\": {
                    \"decorate\": \"(request, response) => { response.headers['X-Decorated'] = 'true'; response.headers['X-Timestamp'] = Date.now().toString(); return response; }\"
                }
            }]
        },"
    done

    stubs="${stubs%,}"
    echo "$stubs"
}

# Create main API imposter (port 4545)
setup_api_imposter() {
    log_info "Setting up API imposter (port 4545) with 500+ stubs..."

    # 10 resources x 10 endpoints each = 100+ stubs, plus extras
    api_stubs=$(generate_api_stubs 10 10)

    config="{
        \"port\": 4545,
        \"protocol\": \"http\",
        \"name\": \"API Server\",
        \"stubs\": [$api_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"

    log_info "API imposter created with $(echo "$api_stubs" | grep -o '"predicates"' | wc -l | tr -d ' ') stubs"
}

# Create regex matching imposter (port 4546)
setup_regex_imposter() {
    log_info "Setting up regex imposter (port 4546) with 100 regex stubs..."

    regex_stubs=$(generate_regex_stubs 100)

    config="{
        \"port\": 4546,
        \"protocol\": \"http\",
        \"name\": \"Regex Matcher\",
        \"stubs\": [$regex_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create complex predicate imposter (port 4547)
setup_complex_imposter() {
    log_info "Setting up complex predicate imposter (port 4547) with 50 complex stubs..."

    complex_stubs=$(generate_complex_predicate_stubs 50)

    config="{
        \"port\": 4547,
        \"protocol\": \"http\",
        \"name\": \"Complex Predicates\",
        \"stubs\": [$complex_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create behavior imposter (port 4548)
setup_behavior_imposter() {
    log_info "Setting up behavior imposter (port 4548) with wait behaviors..."

    behavior_stubs=$(generate_behavior_stubs 20)

    config="{
        \"port\": 4548,
        \"protocol\": \"http\",
        \"name\": \"Behaviors\",
        \"stubs\": [$behavior_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create simple imposter for baseline (port 4549)
setup_simple_imposter() {
    log_info "Setting up simple imposter (port 4549) for baseline..."

    config='{
        "port": 4549,
        "protocol": "http",
        "name": "Simple Baseline",
        "stubs": [{
            "predicates": [{"equals": {"path": "/health"}}],
            "responses": [{"is": {"statusCode": 200, "body": "OK"}}]
        }, {
            "predicates": [{"equals": {"path": "/ping"}}],
            "responses": [{"is": {"statusCode": 200, "body": "pong"}}]
        }]
    }'

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create JSON body matching imposter (port 4550)
setup_json_body_imposter() {
    log_info "Setting up JSON body matching imposter (port 4550) with 100 stubs..."

    json_stubs=$(generate_json_body_stubs 50)

    config="{
        \"port\": 4550,
        \"protocol\": \"http\",
        \"name\": \"JSON Body Matcher\",
        \"stubs\": [$json_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create JSONPath imposter (port 4551)
setup_jsonpath_imposter() {
    log_info "Setting up JSONPath imposter (port 4551) with 50 stubs..."

    jsonpath_stubs=$(generate_jsonpath_stubs 50)

    config="{
        \"port\": 4551,
        \"protocol\": \"http\",
        \"name\": \"JSONPath Matcher\",
        \"stubs\": [$jsonpath_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create XPath imposter (port 4552)
setup_xpath_imposter() {
    log_info "Setting up XPath imposter (port 4552) with 50 stubs..."

    xpath_stubs=$(generate_xpath_stubs 50)

    config="{
        \"port\": 4552,
        \"protocol\": \"http\",
        \"name\": \"XPath Matcher\",
        \"stubs\": [$xpath_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create template response imposter (port 4553)
setup_template_imposter() {
    log_info "Setting up template response imposter (port 4553) with 50 stubs..."

    template_stubs=$(generate_template_stubs 50)

    config="{
        \"port\": 4553,
        \"protocol\": \"http\",
        \"name\": \"Template Responses\",
        \"stubs\": [$template_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create header routing imposter (port 4554)
setup_header_routing_imposter() {
    log_info "Setting up header routing imposter (port 4554) with 100 stubs..."

    header_stubs=$(generate_header_routing_stubs 100)

    config="{
        \"port\": 4554,
        \"protocol\": \"http\",
        \"name\": \"Header Router\",
        \"stubs\": [$header_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create query parameter imposter (port 4555)
setup_query_param_imposter() {
    log_info "Setting up query parameter imposter (port 4555) with 100 stubs..."

    query_stubs=$(generate_query_param_stubs 100)

    config="{
        \"port\": 4555,
        \"protocol\": \"http\",
        \"name\": \"Query Param Matcher\",
        \"stubs\": [$query_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Create decorate behavior imposter (port 4556)
setup_decorate_imposter() {
    log_info "Setting up decorate behavior imposter (port 4556) with 20 stubs..."

    decorate_stubs=$(generate_decorate_stubs 20)

    config="{
        \"port\": 4556,
        \"protocol\": \"http\",
        \"name\": \"Decorate Behaviors\",
        \"stubs\": [$decorate_stubs]
    }"

    create_imposter "$MB_URL" "Mountebank" "$config"
    create_imposter "$RIFT_URL" "Rift" "$config"
}

# Main setup
main() {
    log_info "Starting imposter setup for benchmarking..."
    log_info "Mountebank URL: $MB_URL"
    log_info "Rift URL: $RIFT_URL"
    echo ""

    # Clear existing imposters
    clear_imposters "$MB_URL" "Mountebank"
    clear_imposters "$RIFT_URL" "Rift"

    # Setup all imposters
    setup_simple_imposter
    setup_api_imposter
    setup_regex_imposter
    setup_complex_imposter
    setup_behavior_imposter
    setup_json_body_imposter
    setup_jsonpath_imposter
    setup_xpath_imposter
    setup_template_imposter
    setup_header_routing_imposter
    setup_query_param_imposter
    setup_decorate_imposter

    echo ""
    log_info "All imposters configured successfully!"
    log_info "Summary:"
    echo "  - Port 4545/5545: API Server (~500 stubs)"
    echo "  - Port 4546/5546: Regex Matcher (100 stubs)"
    echo "  - Port 4547/5547: Complex Predicates (50 stubs)"
    echo "  - Port 4548/5548: Behaviors - Wait (20 stubs)"
    echo "  - Port 4549/5549: Simple Baseline (2 stubs)"
    echo "  - Port 4550/5550: JSON Body Matcher (100 stubs)"
    echo "  - Port 4551/5551: JSONPath Matcher (50 stubs)"
    echo "  - Port 4552/5552: XPath Matcher (50 stubs)"
    echo "  - Port 4553/5553: Template Responses (50 stubs)"
    echo "  - Port 4554/5554: Header Router (100 stubs)"
    echo "  - Port 4555/5555: Query Param Matcher (100 stubs)"
    echo "  - Port 4556/5556: Decorate Behaviors (20 stubs)"
    echo ""
    echo "Total: 12 imposters, ~1140+ stubs"
}

main "$@"
