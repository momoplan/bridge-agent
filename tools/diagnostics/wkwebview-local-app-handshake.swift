import Cocoa
import Foundation
import WebKit

private let childURL = URL(string: CommandLine.arguments.dropFirst().first ?? "http://app-handshake.localhost:62555/")!

private final class ParentSchemeHandler: NSObject, WKURLSchemeHandler {
    func webView(_ webView: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
        let html = """
        <!doctype html>
        <html>
          <head>
            <meta charset="utf-8">
            <meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'unsafe-inline'; frame-src http://*.localhost:*">
            <script>
              const report = (event, detail = {}) => {
                window.webkit.messageHandlers.diagnostic.postMessage({ event, ...detail });
              };
              window.addEventListener("message", (event) => {
                const frame = document.getElementById("local-app-frame");
                const expectedOrigin = new URL(frame.src).origin;
                report("message", {
                  data: event.data,
                  actualOrigin: event.origin,
                  expectedOrigin,
                  sourcePresent: event.source !== null,
                  sourceMatches: event.source === frame.contentWindow
                });
              });
              window.addEventListener("DOMContentLoaded", () => report("parent-ready", {
                parentOrigin: window.location.origin
              }));
            </script>
          </head>
          <body>
            <iframe
              id="local-app-frame"
              src="\(childURL.absoluteString)"
              sandbox="allow-forms allow-same-origin allow-scripts"
              referrerpolicy="no-referrer"
              onload="
                const expectedOrigin = new URL(this.src).origin;
                report('iframe-load', { expectedOrigin });
                this.contentWindow.postMessage({ type: 'baijimu:local-app:hello', version: 1 }, expectedOrigin);
              "
            ></iframe>
          </body>
        </html>
        """
        let body = Data(html.utf8)
        let response = URLResponse(
            url: urlSchemeTask.request.url!,
            mimeType: "text/html",
            expectedContentLength: body.count,
            textEncodingName: "utf-8"
        )
        urlSchemeTask.didReceive(response)
        urlSchemeTask.didReceive(body)
        urlSchemeTask.didFinish()
    }

    func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {}
}

private final class DiagnosticHandler: NSObject, WKScriptMessageHandler {
    private var acceptedReady = false

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
        guard JSONSerialization.isValidJSONObject(message.body),
              let data = try? JSONSerialization.data(withJSONObject: message.body, options: [.sortedKeys]),
              let line = String(data: data, encoding: .utf8)
        else {
            print("{\"event\":\"invalid-diagnostic\"}")
            return
        }
        print(line)
        fflush(stdout)

        guard let document = message.body as? [String: Any],
              document["event"] as? String == "message",
              document["sourceMatches"] as? Bool == true,
              document["actualOrigin"] as? String == document["expectedOrigin"] as? String,
              let payload = document["data"] as? [String: Any],
              payload["type"] as? String == "baijimu:local-app:ready"
        else {
            return
        }
        acceptedReady = true
        NSApplication.shared.terminate(nil)
    }

    func finishOnTimeout() {
        if !acceptedReady {
            print("{\"event\":\"timeout\",\"acceptedReady\":false}")
            fflush(stdout)
        }
        NSApplication.shared.terminate(nil)
    }
}

private let app = NSApplication.shared
app.setActivationPolicy(.accessory)

private let schemeHandler = ParentSchemeHandler()
private let diagnosticHandler = DiagnosticHandler()
private let configuration = WKWebViewConfiguration()
configuration.setURLSchemeHandler(schemeHandler, forURLScheme: "tauri")
configuration.userContentController.add(diagnosticHandler, name: "diagnostic")

private let webView = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: configuration)
private let window = NSWindow(
    contentRect: webView.frame,
    styleMask: [.titled, .closable],
    backing: .buffered,
    defer: false
)
window.contentView = webView
window.orderFront(nil)
webView.load(URLRequest(url: URL(string: "tauri://localhost")!))

Timer.scheduledTimer(withTimeInterval: 8, repeats: false) { _ in
    diagnosticHandler.finishOnTimeout()
}

app.run()
