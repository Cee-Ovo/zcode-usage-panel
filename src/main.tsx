import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "open-glass-ui/styles.css";
import "./styles/theme.css";
import "./styles/global.css";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
