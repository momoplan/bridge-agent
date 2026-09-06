    #[test]
    fn connector_asset_content_type_requires_matching_magic_bytes() {
        assert!(content_type_matches_bytes(
            "image/png",
            b"\x89PNG\r\n\x1a\nrest"
        ));
        assert!(content_type_matches_bytes(
            "image/jpeg",
            &[0xff, 0xd8, 0xff, 0xe0]
        ));
        assert!(content_type_matches_bytes(
            "image/webp",
            b"RIFF\x04\x00\x00\x00WEBP"
        ));
        assert!(!content_type_matches_bytes("image/png", b"not an image"));
        assert!(!content_type_matches_bytes(
            "image/jpeg",
            b"\x89PNG\r\n\x1a\n"
        ));
    }

    #[test]
    fn windows_listener_match_accepts_exact_and_unspecified_addresses() {
        let ipv4_bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();
        let ipv6_bind: SocketAddr = "[::1]:18082".parse().unwrap();

        assert!(windows_listener_matches(
            ipv4_bind,
            "127.0.0.1".parse().unwrap(),
            u16::to_be(18081).into()
        ));
        assert!(windows_listener_matches(
            ipv4_bind,
            "0.0.0.0".parse().unwrap(),
            u16::to_be(18081).into()
        ));
        assert!(windows_listener_matches(
            ipv6_bind,
            "::".parse().unwrap(),
            u16::to_be(18082).into()
        ));
        assert!(!windows_listener_matches(
            ipv4_bind,
            "127.0.0.2".parse().unwrap(),
            u16::to_be(18081).into()
        ));
        assert!(!windows_listener_matches(
            ipv4_bind,
            "127.0.0.1".parse().unwrap(),
            u16::to_be(18082).into()
        ));
    }

    #[test]
    fn local_endpoint_does_not_match_different_loopback_address() {
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert!(!local_endpoint_covers_bind("127.0.0.2:18081", bind));
        assert!(!local_endpoint_covers_bind("127.0.0.1:18082", bind));
    }

    #[test]
    fn parse_lsof_owner_matches_loopback_listener() {
        let output = r#"
p1234
c百积木
n127.0.0.1:18081
p5678
cnode
n127.0.0.1:18082
"#;
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert_eq!(
            parse_lsof_listening_owner(output, bind),
            Some(super::OccupiedPortOwner {
                pid: 1234,
                image_name: "百积木".to_string(),
                parent_pid: None,
                executable_path: None,
            })
        );
    }

    #[test]
    fn parse_lsof_owner_treats_unspecified_listener_as_occupying_bind() {
        let output = r#"
p4321
cbridge-agent
n*:18081
"#;
        let bind: SocketAddr = "127.0.0.1:18081".parse().unwrap();

        assert_eq!(
            parse_lsof_listening_owner(output, bind),
            Some(super::OccupiedPortOwner {
                pid: 4321,
                image_name: "bridge-agent".to_string(),
                parent_pid: None,
                executable_path: None,
            })
        );
    }
