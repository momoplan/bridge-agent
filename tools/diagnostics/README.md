# Native local app UI verification

Run `npm run build && npm run test:native` to exercise the real Tauri resource handler on macOS
and Windows. The test creates two isolated local app records, loads HTML and an ES module through
`baijimu-app`, verifies distinct origins and a management bridge round trip, and verifies that child
frames cannot read the host document or access its native IPC. It runs with business startup skipped:
no loopback UI listener or DNS resolution is available or needed. The release frontend gate runs it
on both platforms before signing.

UI resources use native custom schemes on macOS/Linux and Wry-intercepted HTTP-shaped custom
protocol URLs on Windows. The Windows URL is handled inside WebView2; it is not sent to DNS or a
loopback server. The loopback HTTP service is now exclusively the authenticated local CLI control API.

## Historical HTTP handshake reproducer

The following harness reproduces the pre-0.8.6 HTTP loading boundary for diagnostics of older clients.


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
