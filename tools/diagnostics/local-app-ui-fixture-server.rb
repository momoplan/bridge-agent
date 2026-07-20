#!/usr/bin/env ruby

require "webrick"

root = ARGV.fetch(0)
port = Integer(ARGV.fetch(1, "62555"))
bridge_asset = "__baijimu_local_app_bridge__.js"
bridge_script = <<~JS
  (() => {
    const READY_TYPE = "baijimu:local-app:ready";
    const HELLO_TYPE = "baijimu:local-app:hello";
    const announceReady = () => {
      window.parent.postMessage({ type: READY_TYPE, version: 1 }, "*");
    };
    window.addEventListener("message", (event) => {
      if (event.source !== window.parent) return;
      const message = event.data;
      if (message && message.type === HELLO_TYPE && message.version === 1) {
        announceReady();
      }
    });
    Object.defineProperty(window, "baijimuLocalApp", {
      value: Object.freeze({ version: 1 }),
      configurable: false,
      enumerable: true,
      writable: false
    });
    announceReady();
    window.addEventListener("pageshow", announceReady);
  })();
JS

content_types = {
  ".html" => "text/html; charset=utf-8",
  ".css" => "text/css; charset=utf-8",
  ".js" => "application/javascript; charset=utf-8",
  ".mjs" => "application/javascript; charset=utf-8"
}

server = WEBrick::HTTPServer.new(
  BindAddress: "127.0.0.1",
  Port: port,
  AccessLog: [],
  Logger: WEBrick::Log.new($stderr, WEBrick::Log::WARN)
)

server.mount_proc("/") do |request, response|
  relative_path = request.path.sub(%r{\A/}, "")
  relative_path = "index.html" if relative_path.empty?
  if relative_path == bridge_asset
    body = bridge_script
  else
    candidate = File.expand_path(relative_path, root)
    unless candidate.start_with?(File.expand_path(root) + File::SEPARATOR) && File.file?(candidate)
      response.status = 404
      response.body = "not found"
      next
    end
    body = File.binread(candidate)
    if relative_path == "index.html"
      body = body.sub("</head>", %(<script src="./#{bridge_asset}"></script></head>))
    end
  end

  response.status = 200
  response["Content-Type"] = content_types.fetch(File.extname(relative_path), "application/octet-stream")
  response["Cache-Control"] = "no-store"
  response["X-Content-Type-Options"] = "nosniff"
  response["Access-Control-Allow-Origin"] = "*"
  response["Content-Security-Policy"] = "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'none'; object-src 'none'; base-uri 'self'; form-action 'none'; frame-src 'none'; frame-ancestors tauri://localhost http://tauri.localhost http://localhost:1420"
  response.body = body
end

trap("INT") { server.shutdown }
server.start
