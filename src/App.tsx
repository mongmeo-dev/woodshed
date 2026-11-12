import { useState } from "react";
import { SettingsProvider } from "./settings/SettingsProvider";
import { SettingsScreen } from "./settings/SettingsScreen";
import { Sidebar } from "./shell/Sidebar";
import { ToolScreen } from "./shell/ToolScreen";
import { DrumMachineScreen } from "./tools/drumMachine/DrumMachineScreen";
import { MetronomeScreen } from "./tools/metronome/MetronomeScreen";
import type { ViewId } from "./tools/tools";
import { TunerScreen } from "./tools/tuner/TunerScreen";

export function App() {
  const [view, setView] = useState<ViewId>("tuner");
  return (
    <SettingsProvider>
      <div className="shell">
        <Sidebar active={view} onSelect={setView} />
        <main>
          {view === "settings" ? (
            <SettingsScreen />
          ) : view === "metronome" ? (
            <MetronomeScreen />
          ) : view === "tuner" ? (
            <TunerScreen />
          ) : view === "drumMachine" ? (
            <DrumMachineScreen />
          ) : (
            <ToolScreen tool={view} />
          )}
        </main>
      </div>
    </SettingsProvider>
  );
}
