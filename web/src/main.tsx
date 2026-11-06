import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// Lets the app open without signal (see rust/crates/server/static/sw.js).
navigator.serviceWorker?.register("/sw.js").catch(() => {});
