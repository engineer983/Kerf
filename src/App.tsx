import { TitleBar } from "./components/TitleBar";
import { Rail } from "./components/Rail";
import { Canvas } from "./components/Canvas";
import { Inspector } from "./components/Inspector";
import { ChatDrawer } from "./components/ChatDrawer";
import { useUi } from "./state/ui";

export function App() {
  const chatOpen = useUi((s) => s.chatOpen);

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Rail />
        <Canvas />
        <Inspector />
        {chatOpen && <ChatDrawer />}
      </div>
    </div>
  );
}
