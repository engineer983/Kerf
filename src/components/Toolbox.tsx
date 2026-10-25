import { useState } from "react";
import { Plus, Trash2, X } from "lucide-react";
import { fetchPageText, inBrowser, isTauri } from "../lib/ipc";
import {
  parseToolText,
  taperFromDimensions,
  TOOL_KIND_LABELS,
  type Tool,
  type ToolKind,
} from "../lib/tools";
import { useToolbox } from "../state/tools";
import { NumberField, Select } from "./controls";

const field = "w-full rounded-md border border-line bg-raised px-2 py-1.5 select-text";
const quiet = "rounded-lg border border-line px-3 py-1.5 transition-colors hover:text-fg disabled:opacity-40";

/** The bits the user owns, with their dimensions. */
export function Toolbox() {
  const { tools, open, setOpen, add, update, remove } = useToolbox();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  if (!open) return null;
  const selected = tools.find((t) => t.id === selectedId) ?? tools[0] ?? null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-8"
      onClick={() => setOpen(false)}
    >
      <div
        role="dialog"
        aria-label="Toolbox"
        className="flex h-full max-h-[720px] w-full max-w-[900px] flex-col overflow-hidden rounded-2xl border border-line bg-surface shadow-xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-center border-b border-line px-5 py-3">
          <h2 className="text-[14px] font-semibold">Toolbox</h2>
          <span className="ml-3 text-muted">{tools.length} bits</span>
          <button onClick={() => setOpen(false)} aria-label="Close toolbox" className="ml-auto rounded p-1 text-muted hover:text-fg">
            <X size={16} />
          </button>
        </header>

        <div className="flex min-h-0 flex-1">
          <nav className="flex w-64 shrink-0 flex-col border-r border-line">
            <ul className="min-h-0 flex-1 overflow-y-auto p-2">
              {tools.map((tool) => (
                <li key={tool.id}>
                  <button
                    onClick={() => setSelectedId(tool.id)}
                    className={`w-full rounded-lg px-3 py-2 text-left transition-colors ${
                      tool.id === selected?.id ? "bg-raised shadow-sm" : "hover:bg-raised/60"
                    }`}
                  >
                    <div className="truncate">{tool.name}</div>
                    <div className="font-mono text-[11px] text-muted">
                      {TOOL_KIND_LABELS[tool.kind]} · {tool.diameter} mm
                    </div>
                  </button>
                </li>
              ))}
            </ul>
            <button
              onClick={() => setSelectedId(add())}
              className="m-2 flex items-center justify-center gap-1.5 rounded-lg border border-line py-2 text-muted hover:text-fg"
            >
              <Plus size={14} /> Add a bit
            </button>
          </nav>

          {selected ? (
            <ToolForm key={selected.id} tool={selected} update={update} remove={remove} />
          ) : (
            <p className="p-6 text-muted">The toolbox is empty. Add a bit to start.</p>
          )}
        </div>
      </div>
    </div>
  );
}

interface ToolFormProps {
  tool: Tool;
  update: (id: string, patch: Partial<Tool>) => void;
  remove: (id: string) => void;
}

function ToolForm({ tool, update, remove }: ToolFormProps) {
  const set = (patch: Partial<Tool>) => update(tool.id, patch);
  const [pasted, setPasted] = useState("");
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<string | null>(null);

  function apply(text: string, title: string) {
    const { tool: details, found } = parseToolText(text, title);
    if (found.length === 0) {
      setReport("No bit details were recognised in that text.");
      return;
    }
    set(details);
    setReport(`Filled in: ${found.join(", ")}. Check each value against the product before relying on it.`);
  }

  async function fetchPage() {
    setBusy(true);
    setReport(null);
    try {
      const page = await fetchPageText(tool.url.trim());
      apply(page.text, page.title);
    } catch (e) {
      setReport(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col gap-4 overflow-y-auto p-5 [scrollbar-gutter:stable]">
      <label className="flex flex-col gap-1">
        Name
        <input className={field} value={tool.name} onChange={(e) => set({ name: e.target.value })} />
      </label>

      <div className="grid grid-cols-2 gap-x-8 gap-y-3">
        <Select
          label="Type"
          value={tool.kind}
          options={(Object.keys(TOOL_KIND_LABELS) as ToolKind[]).map((kind) => ({
            value: kind,
            label: TOOL_KIND_LABELS[kind],
          }))}
          onChange={(kind) =>
            set({
              kind,
              angle: kind === "vbit" ? (tool.angle ?? 60) : null,
              taperAngle:
                kind === "taperedBall"
                  ? (tool.taperAngle ?? taperFromDimensions(tool.diameter, tool.shank, tool.cutLength))
                  : null,
            })
          }
        />
        <NumberField label="Flutes" min={1} max={8} value={tool.flutes} onChange={(v) => set({ flutes: Math.round(v) })} />
        <NumberField
          label={tool.kind === "taperedBall" ? "Tip diameter" : "Cutting diameter"}
          unit="mm"
          min={0.05}
          max={100}
          value={tool.diameter}
          onChange={(diameter) => set({ diameter })}
        />
        <NumberField label="Cutting length" unit="mm" min={0.1} max={300} value={tool.cutLength} onChange={(cutLength) => set({ cutLength })} />
        <NumberField label="Shank" unit="mm" min={0.5} max={50} value={tool.shank} onChange={(shank) => set({ shank })} />
        {tool.kind === "taperedBall" && (
          <NumberField
            label="Taper per side"
            unit="°"
            min={0.1}
            max={45}
            value={tool.taperAngle ?? taperFromDimensions(tool.diameter, tool.shank, tool.cutLength)}
            onChange={(taperAngle) => set({ taperAngle })}
          />
        )}
        {tool.kind === "vbit" && (
          <NumberField label="Angle" unit="°" min={10} max={150} value={tool.angle ?? 60} onChange={(angle) => set({ angle })} />
        )}
      </div>

      <label className="flex flex-col gap-1">
        Tool number (optional)
        <input
          className={field}
          inputMode="numeric"
          placeholder="None: numbered in the order a job uses it"
          value={tool.number ?? ""}
          onChange={(e) => {
            const typed = e.target.value.trim();
            const number = Math.round(Number(typed));
            set({ number: typed === "" || !Number.isFinite(number) || number < 1 ? null : Math.min(number, 999) });
          }}
        />
        <span className="text-muted">
          Give a number if the bit has a fixed place, such as a slot in a tool rack or an automatic tool changer.
        </span>
      </label>
      <label className="flex flex-col gap-1">
        Brand
        <input className={field} value={tool.brand} onChange={(e) => set({ brand: e.target.value })} />
      </label>
      <label className="flex flex-col gap-1">
        Notes
        <textarea className={`${field} h-16 resize-none`} value={tool.notes} onChange={(e) => set({ notes: e.target.value })} />
      </label>

      <section className="flex flex-col gap-3 rounded-xl border border-line p-4">
        <h3 className="text-[11px] font-semibold uppercase tracking-wider text-muted">
          Fill in from a product listing
        </h3>
        <div className="flex gap-2">
          <input
            aria-label="Product page address"
            placeholder="https://… manufacturer or shop page"
            className={field}
            value={tool.url}
            onChange={(e) => set({ url: e.target.value })}
          />
          <button className={`${quiet} shrink-0 whitespace-nowrap`} disabled={busy || !isTauri() || inBrowser || !tool.url.trim()}
            title={inBrowser ? "A web page cannot read another site's pages. Paste the details below instead, or use the desktop app." : undefined}
            onClick={() => void fetchPage()}
          >
            {busy ? "Reading…" : "Read page"}
          </button>
        </div>
        <textarea
          aria-label="Product details text"
          placeholder="Or paste the title and specifications copied from the listing"
          className={`${field} h-24 resize-none font-mono text-[12px]`}
          value={pasted}
          onChange={(e) => setPasted(e.target.value)}
        />
        <button className={`${quiet} self-start`} disabled={!pasted.trim()} onClick={() => apply(pasted, pasted.split("\n")[0])}>
          Read pasted details
        </button>
        {report && <p className="text-muted">{report}</p>}
        <p className="text-muted">
          Some shops, Amazon among them, refuse automatic page requests. If reading the page fails,
          copy the details from your browser and paste them here.
        </p>
      </section>

      <button
        onClick={() => remove(tool.id)}
        className="flex items-center gap-1.5 self-start text-muted hover:text-accent"
      >
        <Trash2 size={14} /> Remove this bit
      </button>
    </div>
  );
}
