//! Neptune Video's video editor as an MCP server: see `neptune-video-mcp --help`.
fn main() {
    std::process::exit(neptune_video::mcp::run());
}
