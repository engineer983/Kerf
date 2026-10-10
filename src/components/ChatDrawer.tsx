import { X } from "lucide-react";
import { useUi } from "../state/ui";

export function ChatDrawer() {
  const toggleChat = useUi((s) => s.toggleChat);

  return (
    <aside className="flex w-[340px] shrink-0 flex-col border-l border-line bg-surface">
      <div className="flex items-center border-b border-line px-5 py-3">
        <h2 className="text-[13px] font-semibold">Assistant</h2>
        <button
          onClick={toggleChat}
          aria-label="Close assistant"
          className="ml-auto rounded p-1 text-muted hover:text-fg"
        >
          <X size={14} />
        </button>
      </div>
      <p className="p-5 text-muted">
        The assistant is not built yet. It will work with the model provider you choose, and it will
        never be able to move the machine or start a job.
      </p>
    </aside>
  );
}
