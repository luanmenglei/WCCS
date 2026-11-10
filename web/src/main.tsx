import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource/geist/400.css";
import "@fontsource/geist/600.css";
import "./style.css";
import { App } from "./App";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
