import { useEffect } from "react";
import { TitleBar } from "./components/TitleBar";
import { Rail } from "./components/Rail";
import { Canvas } from "./components/Canvas";
import { Inspector } from "./components/Inspector";
import { ChatDrawer } from "./components/ChatDrawer";
import { Toolbox } from "./components/Toolbox";
import { pastePicture } from "./lib/pictures";
import { useUi } from "./state/ui";

export function App() {
  const chatOpen = useUi((s) => s.chatOpen);

  // A picture copied elsewhere, or its web address, can be pasted anywhere.
  useEffect(() => {
    const onPaste = (event: ClipboardEvent) => {
      if (pastePicture(event)) event.preventDefault();
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, []);

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Rail />
        <Canvas />
        <Inspector />
        {chatOpen && <ChatDrawer />}
      </div>
      <Toolbox />
    </div>
  );
}
