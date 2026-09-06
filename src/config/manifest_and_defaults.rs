pub fn manifest_preview_json(config: &AgentConfig) -> Result<String> {
    Ok(serde_json::to_string_pretty(&config.manifest_preview())?)
}

pub fn browser_auth_manifest_json(config: &AgentConfig) -> Result<String> {
    Ok(serde_json::to_string(
        &config.browser_auth_manifest_preview(),
    )?)
}

pub fn resolve_config_base_dir(path: &Path) -> PathBuf {
    path.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true
    })
}

pub fn shell_input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["command"],
        "properties": {
            "command": {
                "description": "Command argv array for direct execution. On Windows, run shell built-ins or PATH lookup through cmd /C, for example [\"cmd\", \"/C\", \"where\", \"wechat-decrypt\"]. For multi-line PowerShell scripts, avoid putting the script body in -Command; pass [\"powershell\", \"-NoProfile\", \"-ExecutionPolicy\", \"Bypass\", \"-File\", \"-\"] and put the script body in stdin.",
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1
            },
            "cwd": {"type": "string"},
            "env": {
                "type": "object",
                "additionalProperties": {"type": "string"}
            },
            "stdin": {
                "description": "Optional text to write to the process standard input. Use this for multi-line scripts instead of embedding long script bodies in command arguments.",
                "type": "string"
            }
        }
    })
}

pub fn shell_execution_id_schema() -> Value {
    json!({
        "type": "object",
        "required": ["executionId"],
        "properties": {
            "executionId": {"type": "string"}
        }
    })
}

pub fn computer_action_input_schema(action: &ComputerUseAction) -> Value {
    match action {
        ComputerUseAction::Screenshot => json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }),
        ComputerUseAction::Click | ComputerUseAction::DoubleClick | ComputerUseAction::Move => {
            json!({
                "type": "object",
                "required": ["x", "y"],
                "properties": {
                    "x": {"type": "number"},
                    "y": {"type": "number"},
                    "button": {
                        "type": "string",
                        "enum": ["left", "middle", "right"]
                    },
                    "keys": {
                        "type": "array",
                        "items": {"type": "string"}
                    }
                }
            })
        }
        ComputerUseAction::Scroll => json!({
            "type": "object",
            "required": ["x", "y"],
            "properties": {
                "x": {"type": "number"},
                "y": {"type": "number"},
                "scroll_x": {"type": "integer"},
                "scroll_y": {"type": "integer"},
                "scrollX": {"type": "integer"},
                "scrollY": {"type": "integer"},
                "keys": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            }
        }),
        ComputerUseAction::Type => json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {"type": "string"}
            }
        }),
        ComputerUseAction::Wait => json!({
            "type": "object",
            "properties": {
                "ms": {
                    "type": "integer",
                    "minimum": 0
                }
            }
        }),
        ComputerUseAction::Keypress => json!({
            "type": "object",
            "required": ["keys"],
            "properties": {
                "keys": {
                    "type": "array",
                    "items": {"type": "string"},
                    "minItems": 1
                }
            }
        }),
        ComputerUseAction::Drag => json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "array",
                    "minItems": 2,
                    "items": {
                        "type": "object",
                        "required": ["x", "y"],
                        "properties": {
                            "x": {"type": "number"},
                            "y": {"type": "number"}
                        }
                    }
                },
                "keys": {
                    "type": "array",
                    "items": {"type": "string"}
                }
            }
        }),
    }
}

fn computer_method(name: &str, description: &str, action: ComputerUseAction) -> MethodConfig {
    MethodConfig {
        name: name.to_string(),
        description: description.to_string(),
        enabled: true,
        input_schema: computer_action_input_schema(&action),
        response_mode: ResponseMode::Cmodel,
        binding: MethodBinding::ComputerUse(ComputerUseBinding {
            action,
            display_id: None,
        }),
    }
}

fn default_computer_service() -> ServiceConfig {
    ServiceConfig {
        name: "computer".to_string(),
        description: "Computer control operations exposed as business methods.".to_string(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: vec![
            computer_method(
                "screenshot",
                "Capture the current desktop and return a PNG screenshot.",
                ComputerUseAction::Screenshot,
            ),
            computer_method(
                "click",
                "Click at a screen coordinate with an optional mouse button.",
                ComputerUseAction::Click,
            ),
            computer_method(
                "double_click",
                "Double-click at a screen coordinate.",
                ComputerUseAction::DoubleClick,
            ),
            computer_method(
                "scroll",
                "Scroll at a screen coordinate with horizontal and vertical deltas.",
                ComputerUseAction::Scroll,
            ),
            computer_method(
                "type",
                "Type text into the currently focused app.",
                ComputerUseAction::Type,
            ),
            computer_method(
                "keypress",
                "Press one key or a key chord such as Command+L.",
                ComputerUseAction::Keypress,
            ),
            computer_method(
                "drag",
                "Drag the pointer across a path of coordinates.",
                ComputerUseAction::Drag,
            ),
            computer_method(
                "move",
                "Move the pointer to a screen coordinate.",
                ComputerUseAction::Move,
            ),
            computer_method(
                "wait",
                "Pause briefly to let the desktop settle before the next screenshot.",
                ComputerUseAction::Wait,
            ),
        ],
    }
}

fn default_shell_service() -> ServiceConfig {
    ServiceConfig {
        name: "shell".to_string(),
        description: "Run and query allowlisted shell commands on the local machine.".to_string(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: default_shell_methods(),
    }
}

fn default_shell_methods() -> Vec<MethodConfig> {
    vec![
        default_shell_exec_method("exec"),
        default_shell_start_execution_method(),
        default_shell_query_execution_method("queryExecution"),
        default_shell_cancel_execution_method(),
    ]
}

fn default_shell_exec_method(name: &str) -> MethodConfig {
    MethodConfig {
        name: name.to_string(),
        description:
            "Run one allowlisted command with optional cwd, env, and stdin. The command is tracked as an execution; quick commands return their result directly, while longer commands return status=RUNNING with executionId and recommendedService/recommendedMethod for polling. For multi-line PowerShell scripts, pass the script via stdin with powershell -File - instead of embedding it in -Command."
                .to_string(),
        enabled: true,
        input_schema: shell_input_schema(),
        response_mode: ResponseMode::Cmodel,
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_start_execution_method() -> MethodConfig {
    MethodConfig {
        name: "startExecution".to_string(),
        description:
            "Start one allowlisted shell command and immediately return an executionId for status polling. Prefer this for installs, downloads, builds, service startup, or any command expected to run longer than 30 seconds. For multi-line PowerShell scripts, pass the script via stdin with powershell -File - instead of embedding it in -Command."
                .to_string(),
        enabled: true,
        input_schema: shell_input_schema(),
        response_mode: ResponseMode::Cmodel,
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_query_execution_method(name: &str) -> MethodConfig {
    MethodConfig {
        name: name.to_string(),
        description:
            "Query status, output, exit code, timing, and errors for a shell executionId returned by exec or startExecution."
            .to_string(),
        enabled: true,
        input_schema: shell_execution_id_schema(),
        response_mode: ResponseMode::Cmodel,
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_cancel_execution_method() -> MethodConfig {
    MethodConfig {
        name: "cancelExecution".to_string(),
        description: "Request cancellation for a running shell executionId.".to_string(),
        enabled: true,
        input_schema: shell_execution_id_schema(),
        response_mode: ResponseMode::Cmodel,
        binding: MethodBinding::ShellCommand(ShellCommandBinding {
            root_dir: ".".to_string(),
            allow_commands: default_shell_exec_allow_commands(),
            default_timeout_secs: Some(default_timeout_secs()),
            max_timeout_secs: Some(default_max_timeout_secs()),
        }),
    }
}

fn default_shell_exec_allow_commands() -> Vec<String> {
    default_shell_exec_core_commands()
        .into_iter()
        .chain(default_shell_exec_runtime_commands())
        .map(str::to_string)
        .collect()
}

fn default_shell_exec_core_commands() -> Vec<&'static str> {
    vec![
        "cmd",
        "powershell",
        "pwsh",
        "sh",
        "bash",
        "echo",
        "pwd",
        "ls",
        "git",
    ]
}

fn default_shell_exec_runtime_commands() -> Vec<&'static str> {
    vec![
        "node", "npm", "npx", "pnpm", "yarn", "python3", "python", "pip3", "pip", "uv",
    ]
}
