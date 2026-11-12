import "./i18n";
import "./styles/global.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { readCachedPrefs } from "./lib/prefsCache";
import { applyTheme } from "./lib/settings";

// Paint the first frame in the last used theme; saved settings follow.
applyTheme(readCachedPrefs()?.theme ?? "system");

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
