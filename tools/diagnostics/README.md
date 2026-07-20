# Local app UI handshake diagnostics

This harness reproduces the production WebKit boundary without starting Bridge Agent. It reports
the child frame origin, whether `event.source` matches the iframe, and whether the ready message is
accepted. It never prints URL tokens or management payloads.

Compile the native harness:

```sh
xcrun swiftc -framework Cocoa -framework WebKit \
  tools/diagnostics/wkwebview-local-app-handshake.swift \
  -o /tmp/wkwebview-local-app-handshake
```

Serve a real local app UI with Bridge Agent's production response headers and injected bridge:

```sh
/usr/bin/ruby tools/diagnostics/local-app-ui-fixture-server.rb \
  "/path/to/connector/package/ui" 62555
```

Run the WebKit probe in another terminal:

```sh
/tmp/wkwebview-local-app-handshake http://app-handshake.localhost:62555/
```

A healthy result contains `sourceMatches: true`, identical actual and expected origins, and a
`baijimu:local-app:ready` message. The harness exits with a timeout record when ready is rejected or
never arrives.

The fixture executable itself is intentionally not bundled as an `.app`. To reproduce macOS App
Transport Security behavior, build the Tauri application bundle and run its executable from
`Contents/MacOS`; inspect the merged `Contents/Info.plist` and the structured `local_app_ui` startup
log records together.
