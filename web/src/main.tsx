import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { Landing } from "./panels/Landing";
import { readUrlOptions, startConnection } from "./state/connection";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("index.html must provide #root");
}

const options = readUrlOptions();
if (options.token !== undefined && options.pipeline !== undefined) {
  startConnection({ token: options.token, pipeline: options.pipeline });
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
} else {
  createRoot(root).render(
    <StrictMode>
      <Landing token={options.token} />
    </StrictMode>,
  );
}
