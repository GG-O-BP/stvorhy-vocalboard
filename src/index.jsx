/* @refresh reload */
import { render } from "@solidjs/web";
import App from "./App.jsx";

render(
  () => <App />,
  /** @type {HTMLElement} */ (document.getElementById("root")),
);
