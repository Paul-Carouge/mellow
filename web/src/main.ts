import { mount } from "svelte";

import App from "./App.svelte";
import { language } from "./i18n";
import "./app.css";

const target = document.getElementById("app");
if (!target) {
  throw new Error("index.html has no #app element");
}
document.documentElement.lang = language;
mount(App, { target });
