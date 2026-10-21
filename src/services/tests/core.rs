    #[test]
    fn allowlist_matches_command_string_not_basename() {
        let allowlist = [String::from("git")];
        assert!(is_command_allowed("git", &allowlist));
        assert!(!is_command_allowed("/usr/bin/git", &allowlist));
        assert!(!is_command_allowed("/tmp/x/git", &allowlist));
        assert!(!is_command_allowed("./git", &allowlist));
        assert!(!is_command_allowed("../git", &allowlist));
        assert!(!is_command_allowed(r"C:\temp\git.exe", &allowlist));
        assert!(!is_command_allowed("bash", &allowlist));
        assert!(is_command_allowed(
            "/usr/bin/git",
            &[String::from("/usr/bin/git")]
        ));
    }

    #[test]
    fn allowlist_accepts_wildcard() {
        assert!(is_command_allowed("bash", &[String::from("*")]));
        assert!(is_command_allowed("/tmp/x/git", &[String::from("*")]));
    }

    #[test]
    fn bgra_conversion_swaps_color_channels_and_rejects_partial_pixels() {
        assert_eq!(
            bgra_to_rgba(&[10, 20, 30, 40, 50, 60, 70, 80]).unwrap(),
            vec![30, 20, 10, 255, 70, 60, 50, 255]
        );
        assert!(bgra_to_rgba(&[10, 20, 30]).is_err());
    }

    #[test]
    fn sanitize_env_keeps_safe_keys_only() {
        let mut env = BTreeMap::new();
        env.insert("FOO_BAR".to_string(), "1".to_string());
        env.insert("NODE_ENV".to_string(), "production".to_string());
        env.insert("bad-key".to_string(), "2".to_string());
        let sanitized = sanitize_env(env);
        assert_eq!(sanitized.get("FOO_BAR"), Some(&"1".to_string()));
        assert_eq!(sanitized.get("NODE_ENV"), Some(&"production".to_string()));
        assert!(!sanitized.contains_key("bad-key"));
    }

    #[test]
    fn sanitize_env_drops_path_and_loader_overrides() {
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), "/tmp/attacker-bin".to_string());
        env.insert("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string());
        env.insert(
            "DYLD_INSERT_LIBRARIES".to_string(),
            "/tmp/evil.dylib".to_string(),
        );
        env.insert(
            "NODE_OPTIONS".to_string(),
            "--require /tmp/evil.js".to_string(),
        );
        env.insert("PYTHONPATH".to_string(), "/tmp/evil".to_string());
        env.insert("GIT_SSH_COMMAND".to_string(), "evil".to_string());
        env.insert("BASH_ENV".to_string(), "/tmp/evil.sh".to_string());
        env.insert("ENV".to_string(), "/tmp/evil.sh".to_string());
        env.insert("HOME".to_string(), "/tmp/attacker-home".to_string());
        env.insert("FOO_BAR".to_string(), "1".to_string());

        let sanitized = sanitize_env(env);
        let path = sanitized.get("PATH").cloned().unwrap_or_default();

        assert_eq!(sanitized.get("FOO_BAR"), Some(&"1".to_string()));
        assert!(!path.contains("/tmp/attacker-bin"));
        assert!(std::env::split_paths(&path).all(|entry| entry.is_absolute()));
        for key in [
            "LD_PRELOAD",
            "DYLD_INSERT_LIBRARIES",
            "NODE_OPTIONS",
            "PYTHONPATH",
            "GIT_SSH_COMMAND",
            "BASH_ENV",
            "ENV",
        ] {
            assert!(!sanitized.contains_key(key), "{key} was forwarded");
        }
        assert_ne!(
            sanitized.get("HOME").map(String::as_str),
            Some("/tmp/attacker-home")
        );
    }

    #[test]
    fn resolve_executable_uses_only_absolute_directories() {
        let dir = tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let file_name = if cfg!(windows) {
            "bridge-tool.exe"
        } else {
            "bridge-tool"
        };
        let tool = bin.join(file_name);
        fs::write(&tool, b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&tool).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&tool, permissions).unwrap();
        }

        let path = std::env::join_paths([&bin]).unwrap();
        let resolved = resolve_executable("bridge-tool", &path.to_string_lossy()).unwrap();
        assert_eq!(resolved, tool);
        assert!(resolve_executable("bridge-tool", ".").is_none());
        assert!(resolve_executable("/tmp/x/bridge-tool", &path.to_string_lossy()).is_none());
    }

    #[test]
    fn shell_exec_path_drops_relative_entries() {
        let path = shell_exec_path(Some("."), None);
        if let Some(path) = path {
            assert!(std::env::split_paths(&path).all(|entry| entry.is_absolute()));
        }
    }

    #[test]
    fn shell_exec_path_prefers_local_toolchains_before_system_path() {
        let dir = tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join(".volta/bin")).unwrap();
        fs::create_dir_all(home.join(".pyenv/shims")).unwrap();
        fs::create_dir_all(home.join(".nvm/versions/node/v18.19.1/bin")).unwrap();
        fs::create_dir_all(home.join(".nvm/versions/node/v20.11.0/bin")).unwrap();
        let system_bin = home.join("system-bin");
        let secondary_bin = home.join("secondary-bin");
        fs::create_dir_all(&system_bin).unwrap();
        fs::create_dir_all(&secondary_bin).unwrap();

        let system_path = std::env::join_paths([
            system_bin.as_path(),
            secondary_bin.as_path(),
            system_bin.as_path(),
        ])
        .unwrap();
        let path = shell_exec_path(Some(&system_path.to_string_lossy()), Some(home)).unwrap();
        let parts = std::env::split_paths(&path).collect::<Vec<_>>();

        assert_eq!(parts[0], home.join(".volta/bin"));
        assert_eq!(parts[1], home.join(".pyenv/shims"));
        assert_eq!(parts[2], home.join(".nvm/versions/node/v20.11.0/bin"));
        assert_eq!(parts[3], home.join(".nvm/versions/node/v18.19.1/bin"));
        assert_eq!(parts.iter().filter(|path| *path == &system_bin).count(), 1);
    }

    #[test]
    fn shell_args_accept_argv_array() {
        let args: ShellExecArgs =
            serde_json::from_value(json!({"command": ["cmd", "/C", "echo", "hello"]})).unwrap();
        assert_eq!(
            args.command,
            ShellCommand::Args(vec![
                "cmd".to_string(),
                "/C".to_string(),
                "echo".to_string(),
                "hello".to_string()
            ])
        );
    }

    #[test]
    fn shell_args_accept_command_line_string() {
        let args: ShellExecArgs =
            serde_json::from_value(json!({"command": "where wechat-decrypt"})).unwrap();
        let command_args = args.command.into_args();

        #[cfg(windows)]
        assert_eq!(
            command_args,
            vec![
                "cmd".to_string(),
                "/C".to_string(),
                "where wechat-decrypt".to_string()
            ]
        );

        #[cfg(not(windows))]
        assert_eq!(
            command_args,
            vec![
                "sh".to_string(),
                "-lc".to_string(),
                "where wechat-decrypt".to_string()
            ]
        );
    }

    #[test]
    fn shell_args_accept_stdin() {
        let args: ShellExecArgs = serde_json::from_value(json!({
            "command": ["sh", "-lc", "cat"],
            "stdin": "bridge-agent-stdin\n"
        }))
        .unwrap();

        assert_eq!(args.stdin.as_deref(), Some("bridge-agent-stdin\n"));
    }

    #[test]
    fn prepare_upload_request_uses_relay_camel_case_schema() {
        let payload = serde_json::to_value(PrepareUploadRequest {
            agent_id: "devbox".to_string(),
            content_type: "image/png".to_string(),
            file_name: "shot.png".to_string(),
            size_bytes: 123,
            workspace_id: Some(42),
            purpose: "computer_screenshot".to_string(),
        })
        .unwrap();

        assert_eq!(payload["agentId"], "devbox");
        assert_eq!(payload["contentType"], "image/png");
        assert_eq!(payload["fileName"], "shot.png");
        assert_eq!(payload["sizeBytes"], 123);
        assert_eq!(payload["workspaceId"], 42);
        assert!(payload.get("agent_id").is_none());
    }

    #[test]
    fn resolve_cwd_rejects_escape() {
        let base = std::env::temp_dir().join(format!("bridge-agent-test-{}", std::process::id()));
        let nested = base.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let root = base.canonicalize().unwrap();
        let escaped = resolve_cwd(&root, Some("../"));
        assert!(escaped.is_err());
        fs::remove_dir_all(&base).unwrap();
    }
