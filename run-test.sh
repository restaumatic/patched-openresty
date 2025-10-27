#!/bin/bash
set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to print colored output
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

# Function to display usage
usage() {
    cat <<EOF
Usage: $(basename "$0") <test-file>

Recompile OpenResty and run a specific Test::Nginx test.

Arguments:
  <test-file>    Path to the .t test file (absolute or relative)

Examples:
  $(basename "$0") openresty-1.27.1.1/bundle/echo-nginx-module-0.63/t/echo.t
  $(basename "$0") /workspace/patched-openresty/openresty-1.27.1.1/bundle/echo-nginx-module-0.63/t/echo.t
  $(basename "$0") bundle/echo-nginx-module-0.63/t/echo.t

Options:
  -h, --help     Display this help message

EOF
    exit 0
}

# Check for help flag
if [[ "${1:-}" == "-h" ]] || [[ "${1:-}" == "--help" ]]; then
    usage
fi

# Check if test file argument is provided
if [[ $# -lt 1 ]]; then
    log_error "Missing test file argument"
    echo
    usage
fi

TEST_FILE="$1"

# Auto-detect repository root
log_info "Detecting repository root..."
REPO_ROOT=$(git rev-parse --show-toplevel 2>/dev/null || true)

if [[ -z "$REPO_ROOT" ]]; then
    log_error "Not inside a git repository. Please run from within the patched-openresty repository."
    exit 1
fi

log_info "Repository root: $REPO_ROOT"

# Resolve test file path (handle both absolute and relative paths)
if [[ "$TEST_FILE" = /* ]]; then
    # Absolute path
    TEST_FILE_FULL="$TEST_FILE"
else
    # Relative path - try multiple possibilities
    if [[ -f "$REPO_ROOT/$TEST_FILE" ]]; then
        TEST_FILE_FULL="$REPO_ROOT/$TEST_FILE"
    elif [[ -f "$REPO_ROOT/openresty-1.27.1.1/$TEST_FILE" ]]; then
        TEST_FILE_FULL="$REPO_ROOT/openresty-1.27.1.1/$TEST_FILE"
    elif [[ -f "$TEST_FILE" ]]; then
        TEST_FILE_FULL="$(realpath "$TEST_FILE")"
    else
        log_error "Test file not found: $TEST_FILE"
        log_error "Tried locations:"
        log_error "  - $REPO_ROOT/$TEST_FILE"
        log_error "  - $REPO_ROOT/openresty-1.27.1.1/$TEST_FILE"
        log_error "  - $TEST_FILE (current directory)"
        exit 1
    fi
fi

# Verify test file exists
if [[ ! -f "$TEST_FILE_FULL" ]]; then
    log_error "Test file does not exist: $TEST_FILE_FULL"
    exit 1
fi

log_success "Found test file: $TEST_FILE_FULL"

# Define paths
OPENRESTY_DIR="$REPO_ROOT/openresty-1.27.1.1"
INSTALL_DIR="$(realpath $REPO_ROOT)/test-install/usr/local/openresty"
NGINX_BINARY="$INSTALL_DIR/nginx/sbin/nginx"
TEST_NGINX_LIB="/workspace/test-nginx/lib"

# Verify OpenResty directory exists
if [[ ! -d "$OPENRESTY_DIR" ]]; then
    log_error "OpenResty directory not found: $OPENRESTY_DIR"
    exit 1
fi

# Verify Test::Nginx library exists
if [[ ! -d "$TEST_NGINX_LIB" ]]; then
    log_warning "Test::Nginx library not found at: $TEST_NGINX_LIB"
    log_warning "Tests may fail if Test::Nginx is not installed in a standard location"
fi

# Step 1: Recompile OpenResty
log_info "Compiling OpenResty with dev-make..."
echo
cd "$OPENRESTY_DIR"

if ./dev-make -j; then
    log_success "Compilation successful"
else
    log_error "Compilation failed"
    exit 1
fi

echo

# Step 1.5: Install to test directory
log_info "Installing to test directory: $INSTALL_DIR"
if make install DESTDIR="$INSTALL_DIR"; then
    log_success "Installation successful"
else
    log_error "Installation failed"
    exit 1
fi

# Verify nginx binary was installed
if [[ ! -f "$NGINX_BINARY" ]]; then
    log_error "Nginx binary not found after installation: $NGINX_BINARY"
    exit 1
fi

log_info "Nginx binary: $NGINX_BINARY"

./generate-symbols "$NGINX_BINARY"

echo

# Step 2: Run the test
log_info "Running test: $(basename "$TEST_FILE_FULL")"
echo

# Set up environment for installed OpenResty
LUAJIT_LIB="$INSTALL_DIR/luajit/lib"
LUALIB_DIR="$INSTALL_DIR/lualib"

log_info "Setting up environment variables for installed modules"

# Set LD_LIBRARY_PATH for LuaJIT shared library
export LD_LIBRARY_PATH="$LUAJIT_LIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
log_info "LD_LIBRARY_PATH: $LD_LIBRARY_PATH"

# Show nginx version
log_info "Nginx version:"
"$NGINX_BINARY" -v 2>&1 | sed 's/^/  /'

# Set LUA_PATH for Lua modules (.lua files)
export LUA_PATH="$LUALIB_DIR/?.lua;$LUALIB_DIR/?/init.lua;;"
log_info "LUA_PATH: $LUA_PATH"

# Set LUA_CPATH for Lua C modules (.so files)
export LUA_CPATH="$LUALIB_DIR/?.so;;"
log_info "LUA_CPATH: $LUA_CPATH"

# Set PATH to use our installed nginx binary
export PATH="$(dirname "$NGINX_BINARY"):$PATH"

# Set TEST_NGINX_BINARY to explicitly use our binary
export TEST_NGINX_BINARY="$NGINX_BINARY"

# Change to the module directory (where the t/ directory is)
# Test::Nginx expects to be run from the module root, not the test file directory
MODULE_DIR=$(dirname "$(dirname "$TEST_FILE_FULL")")
log_info "Changing to module directory: $MODULE_DIR"
cd "$MODULE_DIR"

# Run the test with prove (using relative path from module directory)
TEST_FILE_RELATIVE="t/$(basename "$TEST_FILE_FULL")"
log_info "Running: prove -v -I$TEST_NGINX_LIB $TEST_FILE_RELATIVE"
echo

if prove -v -I"$TEST_NGINX_LIB" "$TEST_FILE_RELATIVE"; then
    echo
    log_success "Test passed!"
    exit 0
else
    echo
    log_error "Test failed!"
    exit 1
fi
