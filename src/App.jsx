import { createSignal, Show } from "solid-js";
import LiveView from "./components/LiveView.jsx";
import SessionsView from "./components/SessionsView.jsx";
import SettingsView from "./components/SettingsView.jsx";

/** @typedef {"live" | "sessions" | "settings"} Tab */

function App() {
  const [tab, setTab] = createSignal(/** @type {Tab} */ ("live"));

  return (
    <div class="app">
      <nav class="tabs">
        <button class={tab() === "live" ? "active" : ""} onClick={() => setTab("live")}>
          라이브
        </button>
        <button class={tab() === "sessions" ? "active" : ""} onClick={() => setTab("sessions")}>
          세션
        </button>
        <button class={tab() === "settings" ? "active" : ""} onClick={() => setTab("settings")}>
          설정
        </button>
      </nav>
      <main class="content">
        <Show when={tab() === "live"}>
          <LiveView />
        </Show>
        <Show when={tab() === "sessions"}>
          <SessionsView />
        </Show>
        <Show when={tab() === "settings"}>
          <SettingsView />
        </Show>
      </main>
    </div>
  );
}

export default App;
