import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { Root } from "./Root";
import { syncConnection } from "./state/connection";
import { installRouting } from "./state/route";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("index.html must provide #root");
}

// The URL is the route (docs/16 §Application layout): `?token=` alone is
// the picker, `&pipeline=` the app on that session, `&view=viewport` the
// pop-out. The connection follows the route — here, outside React, so a
// StrictMode double-mount never opens two sockets — and `Root` renders it.
installRouting(window, syncConnection);
createRoot(root).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
