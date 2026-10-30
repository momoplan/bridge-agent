fn default_file_service() -> ServiceConfig {
    ServiceConfig {
        name: "files".into(),
        description: "Upload and download local files through the file service and object storage. Use files instead of Shell/stdout/Base64 for file transfer. Operations return transferId immediately; poll queryTransfer until SUCCEEDED, FAILED or CANCELLED. File bytes never traverse Relay.".into(),
        enabled: true,
        health_check: None,
        start_command: None,
        stop_command: None,
        methods: vec![
            file_method("upload", "Start uploading a local file to the current authorized user's personal files. Returns transferId; queryTransfer confirms Owner completion and fileRef (fileId, path, name, MIME, size, SHA-256).", file_upload_schema()),
            file_method("download", "Start downloading a personal fileId to an absolute local path. Atomic completion; overwrite defaults to false. Optional expectedSha256 checks content integrity.", file_download_schema()),
            file_method("queryTransfer", "Query status, bytesTransferred, totalBytes, fileRef or error. Tasks survive Relay reconnection, but not a Bridge Agent process restart.", file_transfer_id_schema()),
            file_method("cancelTransfer", "Request cancellation of an unfinished transfer. Owner commit and atomic rename are indivisible; queryTransfer reports the actual terminal result.", file_transfer_id_schema()),
        ],
    }
}
fn file_method(name: &str, description: &str, input_schema: Value) -> MethodConfig {
    MethodConfig {
        name: name.into(),
        description: description.into(),
        enabled: true,
        input_schema,
        response_mode: ResponseMode::Cmodel,
        binding: MethodBinding::FileTransfer,
    }
}
fn file_upload_schema() -> Value {
    json!({
        "type": "object", "required": ["localPath"],
        "properties": {
            "localPath": {"type": "string", "description": "Absolute local file path."},
            "fileName": {"type": "string"},
            "mimeType": {"type": "string", "default": "application/octet-stream"}
        }
    })
}
fn file_download_schema() -> Value {
    json!({
        "type": "object", "required": ["fileId", "localPath"],
        "properties": {
            "fileId": {"type": "string"},
            "localPath": {"type": "string", "description": "Absolute destination path."},
            "overwrite": {"type": "boolean", "default": false},
            "expectedSha256": {"type": "string", "pattern": "^[a-fA-F0-9]{64}$"}
        }
    })
}
fn file_transfer_id_schema() -> Value {
    json!({"type": "object", "required": ["transferId"], "properties": {"transferId": {"type": "string"}}})
}
