import { useEffect, useRef } from "react";
import { TriangleAlert, X } from "lucide-react";
import { HELP_GROUPS, HELP_TOPICS, type HelpBlock } from "../lib/help";
import { useProject } from "../state/project";
import { useUi } from "../state/ui";

/** The guide: how to do things in Kerf, and good practice at each machine. */
export function Help() {
  const topicId = useUi((s) => s.help);
  const machineKind = useProject((s) => s.machineKind);
  const page = useRef<HTMLDivElement>(null);
  const close = () => useUi.getState().setHelp(null);

  useEffect(() => {
    if (!topicId) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [topicId]);

  // Each topic is read from its top.
  useEffect(() => {
    page.current?.scrollTo({ top: 0 });
  }, [topicId]);

  if (!topicId) return null;
  const topic = HELP_TOPICS.find((t) => t.id === topicId) ?? HELP_TOPICS[0];
  // The machine in use comes first of the two; the other is still there to read.
  const yours = machineKind === "laser" ? "Laser" : "Router";
  const other = machineKind === "laser" ? "Router" : "Laser";
  const groups = [...HELP_GROUPS].sort((a, b) => (a === other && b === yours ? 1 : a === yours && b === other ? -1 : 0));

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-8" onClick={close}>
      <div
        role="dialog"
        aria-label="Help"
        className="flex h-full max-h-[760px] w-full max-w-[940px] flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center border-b border-line px-5 py-3">
          <h2 className="text-[14px] font-semibold">Help</h2>
          <span className="ml-3 text-muted">How to do things, and good practice at the machine</span>
          <button onClick={close} aria-label="Close help" className="ml-auto rounded p-1 text-muted hover:text-fg">
            <X size={16} />
          </button>
        </header>

        <div className="flex min-h-0 flex-1">
          <nav aria-label="Help topics" className="w-64 shrink-0 overflow-y-auto border-r border-line p-3">
            {groups.map((group) => (
              <div key={group} className="mb-4">
                <h3 className="px-2 pb-1 text-[11px] font-semibold tracking-wider text-muted uppercase">
                  {group}
                  {group === yours && " · your machine"}
                </h3>
                {HELP_TOPICS.filter((t) => t.group === group).map((t) => (
                  <button
                    key={t.id}
                    onClick={() => useUi.getState().setHelp(t.id)}
                    aria-current={t.id === topic.id ? "page" : undefined}
                    title={`Read: ${t.title}`}
                    className={`block w-full rounded-lg px-2 py-1.5 text-left transition-colors ${
                      t.id === topic.id ? "bg-raised font-medium shadow-sm" : "text-muted hover:text-fg"
                    }`}
                  >
                    {t.title}
                    {/* A guide for the other kind of machine says so. */}
                    {t.machine && t.machine !== machineKind && t.group !== other && (
                      <span className="ml-1 text-[11px] text-muted">({t.machine === "laser" ? "laser" : "router"})</span>
                    )}
                  </button>
                ))}
              </div>
            ))}
          </nav>

          <div ref={page} className="min-w-0 flex-1 overflow-y-auto px-8 py-6 select-text">
            <h3 className="mb-4 text-[18px] font-semibold tracking-tight">{topic.title}</h3>
            <div className="flex max-w-[62ch] flex-col gap-4 leading-relaxed">
              {topic.blocks.map((block, i) => (
                <Block key={i} block={block} />
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

function Block({ block }: { block: HelpBlock }) {
  return (
    <section className="flex flex-col gap-2">
      {block.heading && <h4 className="mt-2 text-[13px] font-semibold">{block.heading}</h4>}
      {block.warning && (
        <p role="note" className="flex gap-2.5 rounded-lg border border-accent/40 px-3 py-2.5">
          <TriangleAlert size={15} className="mt-0.5 shrink-0 text-accent" />
          <span>{block.warning}</span>
        </p>
      )}
      {block.text && <p>{block.text}</p>}
      {block.steps && (
        <ol className="flex list-decimal flex-col gap-1.5 pl-5 marker:font-mono marker:text-[12px] marker:text-muted">
          {block.steps.map((step, i) => (
            <li key={i} className="pl-1">
              {step}
            </li>
          ))}
        </ol>
      )}
      {block.points && (
        <ul className="flex list-disc flex-col gap-1.5 pl-5 marker:text-muted">
          {block.points.map((point, i) => (
            <li key={i} className="pl-1">
              {point}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
