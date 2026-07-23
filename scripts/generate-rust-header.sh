#!/bin/sh
# Regenerates src/openrct2/rustbridge/orct2_agent.h from the Rust crate.
# Run after changing any #[no_mangle] extern "C" item in rust/orct2-agent.
set -eu
cd "$(dirname "$0")/.."
cbindgen --config rust/orct2-agent/cbindgen.toml \
    --crate orct2-agent \
    --output src/openrct2/rustbridge/orct2_agent.h \
    rust/orct2-agent
echo "Regenerated src/openrct2/rustbridge/orct2_agent.h"
