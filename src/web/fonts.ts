/**
 * Fonts for text. A page cannot read the computer's fonts as a program can.
 * Chrome and Edge will hand them over if the person allows it; where they do
 * not, or in another browser, the Lato family that comes with Kerf is used.
 */
import { engine } from "./engine";
import bold from "./fonts/Lato-Bold.ttf?url";
import boldItalic from "./fonts/Lato-BoldItalic.ttf?url";
import italic from "./fonts/Lato-Italic.ttf?url";
import regular from "./fonts/Lato-Regular.ttf?url";

interface LocalFont {
  family: string;
  blob(): Promise<Blob>;
}

const addFont = async (file: Blob | Response) => engine<string[]>("add_font", {}, new Uint8Array(await file.arrayBuffer()));

let included: Promise<unknown> | null = null;

/** Hands the engine the fonts that come with Kerf, the first time text is wanted. */
function loadIncluded(): Promise<unknown> {
  included ??= Promise.all([regular, bold, italic, boldItalic].map(async (url) => addFont(await fetch(url))));
  included.catch(() => (included = null));
  return included;
}

let local: LocalFont[] | null = null;

/**
 * This computer's fonts, if the browser will hand them over. The first time,
 * Chrome asks the person, and only if this follows a click; a refusal, or no
 * click, gives none, and it is asked again next time.
 */
async function localFonts(): Promise<LocalFont[]> {
  if (local) return local;
  const query = (window as unknown as { queryLocalFonts?: () => Promise<LocalFont[]> }).queryLocalFonts;
  if (!query) return [];
  try {
    const found = await query.call(window);
    if (found.length) local = found;
    return found;
  } catch {
    return [];
  }
}

export async function listFonts(): Promise<string[]> {
  await loadIncluded();
  const names = new Set(await engine<string[]>("list_fonts"));
  for (const font of await localFonts()) names.add(font.family);
  return [...names].sort((a, b) => a.toLowerCase().localeCompare(b.toLowerCase()));
}

/** Families already handed to the engine, with the name the engine knows each by. */
const handedOver = new Map<string, string>();

/** Hands the engine a family's font files, and resolves to the name it knows the family by. */
async function family(name: string): Promise<string> {
  const known = handedOver.get(name);
  if (known) return known;
  const files = (await localFonts()).filter((font) => font.family === name);
  if (!files.length) return name;
  const added: string[] = [];
  // The styles of a family can share one file, which need only be handed over once.
  const sizes = new Set<number>();
  for (const font of files) {
    const file = await font.blob();
    if (sizes.has(file.size)) continue;
    sizes.add(file.size);
    added.push(...(await addFont(file)));
  }
  // The file's own name for the family is the one that will be matched.
  const theirs = added.find((a) => a.toLowerCase() === name.toLowerCase()) ?? added[0] ?? name;
  handedOver.set(name, theirs);
  return theirs;
}

export async function makeText({ spec }: { spec: { font: string } }) {
  await loadIncluded();
  return engine("make_text", { spec: { ...spec, font: await family(spec.font) } });
}