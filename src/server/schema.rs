pub(super) fn on_tools_list(msg: &serde_json::Value) -> serde_json::Value {
    let mut tools = file_tools();
    tools.extend(command_tools());
    tools.extend(git_tools());
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": msg["id"],
        "result": { "tools": tools },
    })
}

fn file_tools() -> Vec<serde_json::Value> {
    let mut tools = read_tools();
    tools.extend(write_tools());
    tools.extend(search_info_tools());
    tools
}

fn read_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "read_file",
            "description": "Read the contents of a file, optionally limited to a line range",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "number", "description": "First line to read (1-indexed, inclusive)" },
                    "end_line": { "type": "number", "description": "Last line to read (1-indexed, inclusive)" },
                },
                "required": ["path"],
            },
        }),
        serde_json::json!({
            "name": "list_directory",
            "description": "List entries inside a directory",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        }),
    ]
}

fn write_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "write_file",
            "description": "Write content to a file, creating it if it does not exist",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                },
                "required": ["path", "content"],
            },
        }),
        serde_json::json!({
            "name": "create_directory",
            "description": "Create a directory and any missing parent directories",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        }),
        serde_json::json!({
            "name": "delete_file",
            "description": "Delete a file",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        }),
    ]
}

fn search_info_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "move_file",
            "description": "Move or rename a file or directory",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "from": { "type": "string" },
                    "to": { "type": "string" },
                },
                "required": ["from", "to"],
            },
        }),
        serde_json::json!({
            "name": "search_files",
            "description": "Search for a text pattern across files in a directory",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "pattern": { "type": "string" },
                    "regex": { "type": "boolean", "description": "Treat pattern as a regular expression" },
                },
                "required": ["path", "pattern"],
            },
        }),
        serde_json::json!({
            "name": "get_file_info",
            "description": "Get metadata for a file or directory (size, type, modified time)",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        }),
        serde_json::json!({
            "name": "patch_file",
            "description": "Apply a unified diff patch to a file",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "patch": { "type": "string" },
                },
                "required": ["path", "patch"],
            },
        }),
    ]
}

fn command_tools() -> Vec<serde_json::Value> {
    vec![serde_json::json!({
        "name": "run_command",
        "description": "Run a shell command and return its stdout and stderr",
        "inputSchema": {
            "type": "object",
            "properties": {
                "command": { "type": "string" },
                "cwd": { "type": "string" },
            },
            "required": ["command"],
        },
    })]
}

fn git_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "name": "git_status",
            "description": "Show the working tree status of a git repository",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        }),
        serde_json::json!({
            "name": "git_diff",
            "description": "Show unstaged changes in a git repository",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "staged": { "type": "boolean" },
                },
                "required": ["path"],
            },
        }),
        serde_json::json!({
            "name": "git_log",
            "description": "Show recent commits in a git repository",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "count": { "type": "number" },
                },
                "required": ["path"],
            },
        }),
        serde_json::json!({
            "name": "git_commit",
            "description": "Stage all changes and create a git commit with the given message",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "message": { "type": "string" },
                },
                "required": ["path", "message"],
            },
        }),
    ]
}
