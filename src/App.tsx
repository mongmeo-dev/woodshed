import { useState } from "react";
import { SettingsProvider } from "./settings/SettingsProvider";
import { SettingsScreen } from "./settings/SettingsScreen";
import { Sidebar } from "./shell/Sidebar";
import { ToolScreen } from "./shell/ToolScreen";
import { DrumMachineScreen } from "./tools/drumMachine/DrumMachineScreen";
import { FretboardScreen } from "./tools/fretboard/FretboardScreen";
import { MetronomeScreen } from "./tools/metronome/MetronomeScreen";
import { StemSplitterScreen } from "./tools/stemSplitter/StemSplitterScreen";
import type { ViewId } from "./tools/tools";
import { TunerScreen } from "./tools/tuner/TunerScreen";
import { UpdaterProvider } from "./updater/UpdaterProvider";

export function App() {
  const [view, setView] = useState<ViewId>("tuner");
  return (
    <SettingsProvider>
      <UpdaterProvider>
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
            ) : view === "stemSplitter" ? (
              <StemSplitterScreen />
            ) : view === "fretboard" ? (
              <FretboardScreen />
            ) : (
              <ToolScreen tool={view} />
            )}
          </main>
        </div>
      </UpdaterProvider>
    </SettingsProvider>
  );
}
