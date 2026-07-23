# Rust agent bridge: builds rust/orct2-agent as a staticlib via Corrosion and
# exposes the `orct2-agent` target for libopenrct2 to link against.
include(FetchContent)
FetchContent_Declare(
    Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG v0.6.1
)
FetchContent_MakeAvailable(Corrosion)

corrosion_import_crate(
    MANIFEST_PATH "${ROOT_DIR}/rust/orct2-agent/Cargo.toml"
    CRATE_TYPES staticlib
)
