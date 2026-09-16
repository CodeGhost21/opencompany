use super::*;

fn parse(src: &str) -> (Vec<McpServer>, Vec<String>) {
    parse_mcp_file(MCP_FILE, src)
}

#[test]
fn reads_a_server_and_takes_its_name_from_the_key() {
    let (servers, problems) = parse(
        r#"{"mcpServers": {"deepwiki": {
            "url": "https://mcp.deepwiki.com/mcp",
            "description": "Docs for public repos."
