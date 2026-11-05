import { mount } from "svelte";

import App from "./App.svelte";
import { language, loadStrings } from "./i18n";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("index.html has no #app element");
}
document.documentElement.lang = language;
// Not a top-level await: Rolldown stops merging common chunks when a module uses one, which
// split the startup code into dozens of small files that compress worse.
void loadStrings().then(() => mount(App, { target }));
