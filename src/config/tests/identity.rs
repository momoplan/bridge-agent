    #[test]
    fn browser_auth_migrates_legacy_default_agent_id() {
        let mut config = AgentConfig::example();
        config.relay.agent_id = "devbox".to_string();

        let changed = ensure_browser_auth_agent_id(&mut config);

        assert!(changed);
        assert_generated_agent_id(&config.relay.agent_id);
    }

    #[test]
    fn browser_auth_keeps_existing_custom_agent_id() {
        let mut config = AgentConfig::example();
        config.relay.agent_id = "dev_my_custom_box".to_string();

        let changed = ensure_browser_auth_agent_id(&mut config);

        assert!(!changed);
        assert_eq!(config.relay.agent_id, "dev_my_custom_box");
    }
