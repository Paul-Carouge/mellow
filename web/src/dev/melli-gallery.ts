import { mount } from "svelte";

import "../app.css";
import MelliGallery from "./MelliGallery.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("melli.html has no #app element");
}
mount(MelliGallery, { target });
