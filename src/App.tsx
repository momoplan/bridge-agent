import { AppView } from "./AppView";
import { buildConsoleUrl, fromUiConfig, normalizePlatformBaseUrl, toUiConfig } from "./app/config-conversion";
import { needsBrowserAuthorization } from "./app/formatters";
import { useAppController } from "./use-app-controller";

function App() {
  return <AppView controller={useAppController()} />;
}

export default App;

export { buildConsoleUrl, fromUiConfig, needsBrowserAuthorization, normalizePlatformBaseUrl, toUiConfig };
