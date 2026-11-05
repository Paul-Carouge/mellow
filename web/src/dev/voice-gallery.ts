import { mount } from "svelte";

import "../app.css";
import { language, loadStrings } from "../i18n";
import VoiceGallery from "./VoiceGallery.svelte";

const target = document.getElementById("app");
if (!target) {
  throw new Error("voice.html has no #app element");
}
document.documentElement.lang = language;
void loadStrings().then(() => mount(VoiceGallery, { target }));
