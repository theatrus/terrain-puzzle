import React from "react";
import { createRoot } from "react-dom/client";
import "../app/globals.css";
import { TerrainStudio } from "../app/terrain-studio";

const root = document.getElementById("root");
if (!root) throw new Error("desktop root element is missing");

createRoot(root).render(
  <React.StrictMode>
    <TerrainStudio />
  </React.StrictMode>,
);
