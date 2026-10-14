import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import ErrorBoundary from "./components/ErrorBoundary";
import { logUiError } from "./lib/api";
import QuickCapture from "./views/QuickCapture";
import "./index.css";

// One bundle, two windows: the menu-bar popover ("quick") and the full app.
const isQuick = getCurrentWindow().label === "quick";

// Errors outside React rendering (event handlers, promises) go to the log too.
window.addEventListener("error", (e) => logUiError(`${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => logUiError(`unhandled rejection: ${String(e.reason)}`));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>{isQuick ? <QuickCapture /> : <App />}</ErrorBoundary>
  </React.StrictMode>,
);
