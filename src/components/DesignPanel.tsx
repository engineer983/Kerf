import { describeDesign } from "../lib/design";
import { useProject } from "../state/project";
import { useUi } from "../state/ui";
import { Section, Segmented } from "./controls";

const quiet = "rounded-lg border border-line px-3 py-1.5 transition-colors hover:text-accent";

/** What the design on the canvas is, which sheet of it is showing, and how to change it. */
export function DesignPanel() {
  const design = useProject((s) => s.design);
  const machineKind = useProject((s) => s.machineKind);
  if (!design) return null;
  const sheet = design.sheets[design.sheet];
  const tidy = (mm: number) => Number(mm.toFixed(1));

  return (
    <Section title={design.kind === "box" ? "Box" : "Stacked layers"}>
      <p>{describeDesign(design)}</p>
      {design.sheets.length > 1 && (
        <>
          <p className="text-muted">
            It does not fit the machine in one go, so it is cut a sheet at a time. Generate and cut each
            sheet in turn.
          </p>
          <Segmented
            label="Sheet to cut now"
            value={String(design.sheet)}
            options={design.sheets.map((_, i) => ({ value: String(i), label: `Sheet ${i + 1}` }))}
            onChange={(i) => void useProject.getState().setDesignSheet(Number(i))}
          />
        </>
      )}
      <p className="text-muted">
        This sheet is {tidy(sheet.widthMm)} × {tidy(sheet.heightMm)} mm and holds the {sheet.parts.join(", ")}.
      </p>
      {design.madeFor !== machineKind && (
        <p role="alert" className="text-accent">
          This was drawn for a {design.madeFor === "laser" ? "laser's" : "router bit's"} width of cut. Make it
          again so it suits the machine now chosen.
        </p>
      )}
      <p className="text-muted">
        {machineKind === "laser"
          ? "The power and speed that cut through depend on your laser and material, so they are yours to set. Try them on an offcut first."
          : "The cut goes the full thickness of the material. Stick the sheet down and put scrap board underneath."}
      </p>
      <div className="flex flex-wrap gap-2">
        <button className={quiet} onClick={() => useUi.getState().setMaker(design.kind)}>
          {design.kind === "box" ? "Change the box…" : "Change the layers…"}
        </button>
        {design.kind === "layers" && (
          <button className={quiet} onClick={() => void useProject.getState().clearDesign()}>
            Back to the picture
          </button>
        )}
      </div>
    </Section>
  );
}
