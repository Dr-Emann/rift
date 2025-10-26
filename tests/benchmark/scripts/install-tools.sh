#!/bin/bash
# Install required tools for running benchmarks
#
# Supported platforms: macOS, Linux (Debian/Ubuntu, Fedora/RHEL, Alpine)

set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Detect OS
detect_os() {
    if [[ "$OSTYPE" == "darwin"* ]]; then
        echo "macos"
    elif [[ -f /etc/debian_version ]]; then
        echo "debian"
    elif [[ -f /etc/fedora-release ]]; then
        echo "fedora"
    elif [[ -f /etc/alpine-release ]]; then
        echo "alpine"
    elif [[ -f /etc/redhat-release ]]; then
        echo "rhel"
    else
        echo "unknown"
    fi
}

# Install hey - HTTP load generator
install_hey() {
    if command -v hey &> /dev/null; then
        log_info "hey is already installed: $(hey -version 2>/dev/null || echo 'version unknown')"
        return 0
    fi

    log_info "Installing hey..."

    local os=$(detect_os)

    case $os in
        macos)
            if command -v brew &> /dev/null; then
                brew install hey
            else
                # Download binary directly
                curl -sSL https://hey-release.s3.us-east-2.amazonaws.com/hey_darwin_amd64 -o /usr/local/bin/hey
                chmod +x /usr/local/bin/hey
            fi
            ;;
        debian)
            # hey is written in Go, install via go or download binary
            if command -v go &> /dev/null; then
                go install github.com/rakyll/hey@latest
            else
                log_info "Installing via binary download..."
                sudo curl -sSL https://hey-release.s3.us-east-2.amazonaws.com/hey_linux_amd64 -o /usr/local/bin/hey
                sudo chmod +x /usr/local/bin/hey
            fi
            ;;
        fedora|rhel)
            if command -v go &> /dev/null; then
                go install github.com/rakyll/hey@latest
            else
                sudo curl -sSL https://hey-release.s3.us-east-2.amazonaws.com/hey_linux_amd64 -o /usr/local/bin/hey
                sudo chmod +x /usr/local/bin/hey
            fi
            ;;
        alpine)
            if command -v go &> /dev/null; then
                go install github.com/rakyll/hey@latest
            else
                wget -O /usr/local/bin/hey https://hey-release.s3.us-east-2.amazonaws.com/hey_linux_amd64
                chmod +x /usr/local/bin/hey
            fi
            ;;
        *)
            log_error "Unknown OS. Please install hey manually: https://github.com/rakyll/hey"
            return 1
            ;;
    esac

    log_info "hey installed successfully"
}

# Install jq - JSON processor
install_jq() {
    if command -v jq &> /dev/null; then
        log_info "jq is already installed: $(jq --version)"
        return 0
    fi

    log_info "Installing jq..."

    local os=$(detect_os)

    case $os in
        macos)
            if command -v brew &> /dev/null; then
                brew install jq
            else
                curl -sSL -o /usr/local/bin/jq https://github.com/stedolan/jq/releases/download/jq-1.7/jq-macos-amd64
                chmod +x /usr/local/bin/jq
            fi
            ;;
        debian)
            sudo apt-get update && sudo apt-get install -y jq
            ;;
        fedora|rhel)
            sudo dnf install -y jq || sudo yum install -y jq
            ;;
        alpine)
            apk add --no-cache jq
            ;;
        *)
            log_error "Unknown OS. Please install jq manually"
            return 1
            ;;
    esac

    log_info "jq installed successfully"
}

# Install curl
install_curl() {
    if command -v curl &> /dev/null; then
        log_info "curl is already installed: $(curl --version | head -1)"
        return 0
    fi

    log_info "Installing curl..."

    local os=$(detect_os)

    case $os in
        macos)
            # curl is pre-installed on macOS
            log_info "curl should be pre-installed on macOS"
            ;;
        debian)
            sudo apt-get update && sudo apt-get install -y curl
            ;;
        fedora|rhel)
            sudo dnf install -y curl || sudo yum install -y curl
            ;;
        alpine)
            apk add --no-cache curl
            ;;
        *)
            log_error "Unknown OS. Please install curl manually"
            return 1
            ;;
    esac

    log_info "curl installed successfully"
}

# Install bc (calculator for bash)
install_bc() {
    if command -v bc &> /dev/null; then
        log_info "bc is already installed"
        return 0
    fi

    log_info "Installing bc..."

    local os=$(detect_os)

    case $os in
        macos)
            # bc is pre-installed on macOS
            log_info "bc should be pre-installed on macOS"
            ;;
        debian)
            sudo apt-get update && sudo apt-get install -y bc
            ;;
        fedora|rhel)
            sudo dnf install -y bc || sudo yum install -y bc
            ;;
        alpine)
            apk add --no-cache bc
            ;;
        *)
            log_warn "bc not found. Some calculations may not work."
            ;;
    esac
}

# Verify Docker is installed
check_docker() {
    if ! command -v docker &> /dev/null; then
        log_error "Docker is not installed. Please install Docker first:"
        echo "  - macOS: https://docs.docker.com/desktop/mac/install/"
        echo "  - Linux: https://docs.docker.com/engine/install/"
        return 1
    fi

    if ! command -v docker compose &> /dev/null; then
        log_error "Docker Compose is not installed. Please install Docker Compose:"
        echo "  https://docs.docker.com/compose/install/"
        return 1
    fi

    log_info "Docker is installed: $(docker --version)"
    log_info "Docker Compose: $(docker compose version)"
}

# Main
main() {
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║      Benchmark Tools Installation Script                   ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo ""

    local os=$(detect_os)
    log_info "Detected OS: $os"
    echo ""

    # Check Docker first
    check_docker

    echo ""
    log_info "Installing benchmark tools..."
    echo ""

    install_curl
    install_jq
    install_bc
    install_hey

    echo ""
    log_info "All tools installed successfully!"
    echo ""
    echo "Installed tools:"
    echo "  - curl: $(curl --version 2>/dev/null | head -1 || echo 'not found')"
    echo "  - jq: $(jq --version 2>/dev/null || echo 'not found')"
    echo "  - hey: $(which hey 2>/dev/null || echo 'not found')"
    echo "  - bc: $(which bc 2>/dev/null || echo 'not found')"
    echo ""
    echo "You can now run the benchmarks with:"
    echo "  cd tests/benchmark"
    echo "  docker compose up -d --build"
    echo "  ./scripts/run-benchmark.sh"
}

main "$@"
