import { defaultGoal, goalsFor, useProject, type Goal, type Quality } from "../state/project";
import { useToolbox } from "../state/tools";
import { useUi } from "../state/ui";
import { Checkbox, NumberField, Section, Segmented } from "./controls";
import { OptimizeButton } from "./OptimizeButton";

const GOALS: Record<Goal, { label: string; about: string }> = {
  picture: { label: "Engrave the picture", about: "Burns the picture with its shading, like a photograph." },
  shapes: { label: "Bold shapes", about: "Fills the picture's dark areas solidly. Best for logos and text." },
  outline: { label: "Outline only", about: "Follows the edges of the shapes with a single line." },
  carving: { label: "3D carving", about: "Carves the picture as a raised shape you can feel." },
  lines: { label: "Follow the lines", about: "Runs the bit along the edges of the shapes." },
};

const QUALITY_ABOUT: Record<Quality, string> = {
  draft: "Quickest. Coarser, for a trial run.",
  standard: "A balance of time and detail.",
  fine: "The most detail, and the longest job.",
};

/** A few choices that set the whole job up, in place of the individual settings. */
export function SimpleSetup() {
  const machineKind = useProject((s) => s.machineKind);
  const photo = useProject((s) => s.image?.photo ?? false);
  // Words and drawings are lines: there is no shading to engrave or height to carve.
  const drawn = useProject((s) => s.design !== null);
  const simple = useProject((s) => s.simple);
  const operations = useProject((s) => s.operations);
  const depth = useProject((s) => s.depth);
  const thickness = useProject((s) => s.thickness);
  const tools = useToolbox((s) => s.tools);
  const hasItems = useProject((s) => s.items.length > 0);
  const { setSimple, updateOperation, setOperationType } = useProject.getState();

  const goal = simple.goal && goalsFor(machineKind, drawn).includes(simple.goal) ? simple.goal : defaultGoal(machineKind, photo, drawn);
  // The main artwork's operations; whatever makes the added text and drawings is set separately.
  const [main, cut] = operations.filter((op) => !op.forItems);
  const added = hasItems ? operations.find((op) => op.forItems) : undefined;
  // On a laser, the cut around the piece has its own power and speed to set.
  const laserCut = cut?.kind.type === "laserCutout" ? { ...cut, kind: cut.kind } : null;
  const bitName = (toolId?: string | null) => tools.find((t) => t.id === toolId)?.name;
  // The carving and the cut-out are separate runs when they need different bits.
  const twoParts = Boolean(cut && main && main.toolId !== cut.toolId);

  return (
    <>
      <Section title="What do you want to make?">
        {goalsFor(machineKind, drawn).map((id) => (
          <button
            key={id}
            onClick={() => setSimple({ goal: id })}
            title={GOALS[id].about}
            className={`flex w-full flex-col rounded-xl border p-3 text-left transition-colors ${
              id === goal ? "border-accent bg-raised shadow-sm" : "border-line hover:border-muted"
            }`}
          >
            <span className="font-medium">{GOALS[id].label}</span>
            <span className="text-muted">{GOALS[id].about}</span>
          </button>
        ))}
        {!drawn && (
          <>
            <button
              onClick={() => useUi.getState().setMaker("layers")}
              title="Slice the picture into layers to cut from thin sheet and stack"
              className="flex w-full flex-col rounded-xl border border-line p-3 text-left transition-colors hover:border-muted"
            >
              <span className="font-medium">Stacked layers…</span>
              <span className="text-muted">Cuts the picture as layers to glue into a piece with real depth.</span>
            </button>
            <OptimizeButton />
          </>
        )}
      </Section>

      <Section title="How fine?">
        <Segmented
          label="Quality"
          value={simple.quality}
          options={[
            { value: "draft", label: "Draft" },
            { value: "standard", label: "Standard" },
            { value: "fine", label: "Fine" },
          ]}
          onChange={(quality) => setSimple({ quality })}
        />
        <p className="text-muted">{QUALITY_ABOUT[simple.quality]}</p>

        {goal === "carving" && (
          <>
            <NumberField
              label="Depth"
              unit="mm"
              min={0.5}
              max={Math.max(0.5, thickness - 2)}
              value={Math.min(simple.depth, Math.max(0.5, thickness - 2))}
              onChange={(value) => setSimple({ depth: value })}
            />
            {depth.status === "working" && <p className="animate-pulse text-muted">{depth.message ?? "Working out the shape…"}</p>}
            {depth.status === "error" && (
              <p role="alert" className="text-accent">
                The picture's shape could not be worked out ({depth.message}), so the carving follows its
                brightness instead.
              </p>
            )}
          </>
        )}

        {machineKind === "laser" && main && (
          <>
            <NumberField label="Power" unit="%" min={0} max={100} value={main.power} onChange={(power) => updateOperation(main.id, { power })} />
            <NumberField label="Speed" unit="mm/min" min={1} max={100000} value={main.feed} onChange={(feed) => updateOperation(main.id, { feed })} />
            <p className="text-muted">
              Power and speed depend on your laser and the material, so they are yours to set. Try a
              small patch on scrap first.
            </p>
          </>
        )}

        {machineKind === "cnc" ? (
          <Checkbox
            label="Cut the piece out when done"
            hint={{ text: "Adds a cut around the artwork, right through the material, leaving small tabs so the piece stays put. Put scrap board underneath.", demo: "tabs" }}
            checked={simple.cutOut}
            onChange={(cutOut) => setSimple({ cutOut })}
          />
        ) : (
          <>
            <Checkbox
              label="Cut the piece out when done"
              hint={{ text: "Adds a cut around the artwork, right through the material, to free the piece. It runs in the same job, straight after the engraving." }}
              checked={simple.cutOut}
              onChange={(cutOut) => setSimple({ cutOut })}
            />
            {laserCut && (
              <>
                <NumberField
                  label="Cut power"
                  unit="%"
                  min={0}
                  max={100}
                  value={laserCut.power}
                  hint={{ text: "How strongly the laser fires for the cut around the piece. Cutting through usually takes full power." }}
                  onChange={(power) => updateOperation(laserCut.id, { power })}
                />
                <NumberField
                  label="Cut speed"
                  unit="mm/min"
                  min={1}
                  max={100000}
                  value={laserCut.feed}
                  hint={{ text: "How fast the laser moves for the cut around the piece. Slower cuts deeper, and chars more." }}
                  onChange={(feed) => updateOperation(laserCut.id, { feed })}
                />
                <NumberField
                  label="Cut passes"
                  step={1}
                  min={1}
                  max={100}
                  value={laserCut.passes}
                  hint={{ text: "How many times the laser goes round the cut. Several faster passes often get through more cleanly than one slow one." }}
                  onChange={(passes) => updateOperation(laserCut.id, { passes: Math.round(passes) })}
                />
                <NumberField
                  label="Gap around the artwork"
                  unit="mm"
                  min={-1000}
                  max={1000}
                  value={laserCut.kind.offset}
                  hint={{ text: "How far outside the artwork the cut runs. A negative number cuts inside its edge.", demo: "offset" }}
                  onChange={(offset) => updateOperation(laserCut.id, { kind: { ...laserCut.kind, offset } })}
                />
                <p className="text-muted">
                  The cut's power and speed depend on your laser and the material's thickness. Test on
                  an offcut until it just falls free.
                </p>
              </>
            )}
          </>
        )}
      </Section>

      {machineKind === "cnc" && main && (
        <Section title="Bit to fit">
          {twoParts ? (
            <>
              <p className="text-muted">
                The two parts need different bits, so they are made one after the other. Make the first,
                then change the bit, set the height again, and make the second.
              </p>
              <Segmented
                label="Part to make now"
                value={simple.part}
                options={[
                  { value: "main", label: "1. Main part" },
                  { value: "cutOut", label: "2. Cut-out" },
                ]}
                onChange={(part) => setSimple({ part })}
              />
              <p>
                Fit the <strong>{bitName(simple.part === "main" ? main.toolId : cut.toolId) ?? "chosen bit"}</strong>.
              </p>
            </>
          ) : (
            <p>
              Fit the <strong>{bitName(main.toolId) ?? "bit of your choice"}</strong>.
            </p>
          )}
          <p className="text-muted">Bits come from your toolbox. Speeds are set to suit the bit and the material.</p>
        </Section>
      )}
      {added && (
        <Section title="Added text and drawings">
          {added.kind.type === "laserFill" || added.kind.type === "laserCut" ? (
            <>
              <Segmented
                label="How added text and drawings are made"
                value={added.kind.type}
                options={[
                  { value: "laserFill", label: "Engrave solid" },
                  { value: "laserCut", label: "Outline" },
                ]}
                onChange={(type) => setOperationType(added.id, type)}
              />
              <NumberField
                label="Text power"
                unit="%"
                min={0}
                max={100}
                value={added.power}
                hint={{ text: "How strongly the laser fires for the text and drawings you added. It is set apart from the main artwork's power." }}
                onChange={(power) => updateOperation(added.id, { power })}
              />
              <NumberField
                label="Text speed"
                unit="mm/min"
                min={1}
                max={100000}
                value={added.feed}
                hint={{ text: "How fast the laser moves for the text and drawings you added." }}
                onChange={(feed) => updateOperation(added.id, { feed })}
              />
            </>
          ) : added.kind.type === "cncProfile" ? (
            <>
              <NumberField
                label="Text depth"
                unit="mm"
                min={0.01}
                max={200}
                value={added.kind.depth}
                hint={{ text: "How deep the bit cuts along the edges of the text and drawings you added." }}
                onChange={(depth) => updateOperation(added.id, { kind: { ...added.kind, depth } as typeof added.kind })}
              />
              <p className="text-muted">
                Cut with the {bitName(added.toolId) ?? "same bit"}, following the edge of each letter with the
                middle of the bit.
              </p>
            </>
          ) : null}
          <p className="text-muted">They are made in the same job, after the main artwork and before any cut-out.</p>
        </Section>
      )}
    </>
  );
}
