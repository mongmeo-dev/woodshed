import { useState } from "react";
import { SettingsProvider } from "./settings/SettingsProvider";
import { SettingsScreen } from "./settings/SettingsScreen";
import { Sidebar } from "./shell/Sidebar";
import { ToolScreen } from "./shell/ToolScreen";
import { DrumMachineScreen } from "./tools/drumMachine/DrumMachineScreen";
import { FretboardScreen } from "./tools/fretboard/FretboardScreen";
import type { ViewId } from "./tools/tools";

export function App() {
  const [view, setView] = useState<ViewId>("tuner");
  return (
    <SettingsProvider>
      <div className="shell">
        <Sidebar active={view} onSelect={setView} />
        <main>
          {view === "settings" ? (
            <SettingsScreen />
          ) : view === "drumMachine" ? (
            <DrumMachineScreen />
          ) : view === "fretboard" ? (
            <FretboardScreen />
          ) : (
            <ToolScreen tool={view} />
          )}
        </main>
      </div>
    </SettingsProvider>
  );
}
