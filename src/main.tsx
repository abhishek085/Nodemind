import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import App from "./App";
import QuickCapture from "./views/QuickCapture";
import "./index.css";

// One bundle, two windows: the menu-bar popover ("quick") and the full app.
const isQuick = getCurrentWindow().label === "quick";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isQuick ? <QuickCapture /> : <App />}</React.StrictMode>,
);
