    #[test]
    fn allowlist_accepts_basename() {
        assert!(is_command_allowed("/usr/bin/git", &[String::from("git")]));
        assert!(!is_command_allowed("bash", &[String::from("git")]));
    }

    #[test]
    fn allowlist_accepts_wildcard() {
        assert!(is_command_allowed("bash", &[String::from("*")]));
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
        env.insert("bad-key".to_string(), "2".to_string());
        let sanitized = sanitize_env(env);
        assert_eq!(sanitized.get("FOO_BAR"), Some(&"1".to_string()));
        assert!(!sanitized.contains_key("bad-key"));
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
