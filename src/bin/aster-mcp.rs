//! Aster's video editor as an MCP server: see `aster-mcp --help`.
fn main() {
    std::process::exit(aster_editor::mcp::run());
}
