/** Saving and opening files from a web page. */
import { engine } from "./engine";

interface FileHandle {
  name: string;
  getFile(): Promise<File>;
  createWritable(): Promise<{ write(data: string): Promise<void>; close(): Promise<void> }>;
}

interface FileType {
  description: string;
  accept: Record<string, string[]>;
}

// Chrome and Edge let a page ask where to save; other browsers only download.
const pickers = window as unknown as {
  showSaveFilePicker?: (options: { suggestedName: string; types: FileType[] }) => Promise<FileHandle>;
  showOpenFilePicker?: (options: { types: FileType[] }) => Promise<FileHandle[]>;
};

const cancelled = (e: unknown) => e instanceof DOMException && e.name === "AbortError";

function download(name: string, text: string, mime: string) {
  const url = URL.createObjectURL(new Blob([text], { type: mime }));
  const link = document.createElement("a");
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/**
 * Saves text the person asked for. The place is asked for before the text is
 * worked out, because a browser only lets a page ask straight after a click.
 * Resolves to where it went, or null if the person cancelled.
 */
async function save(suggestedName: string, type: FileType, text: () => Promise<string>): Promise<string | null> {
  const mime = Object.keys(type.accept)[0];
  if (pickers.showSaveFilePicker) {
    let handle: FileHandle | null = null;
    try {
      handle = await pickers.showSaveFilePicker({ suggestedName, types: [type] });
    } catch (e) {
      if (cancelled(e)) return null;
      // Refused for some other reason: the file is downloaded instead.
    }
    if (handle) {
      const writable = await handle.createWritable();
      await writable.write(await text());
      await writable.close();
      return handle.name;
    }
  }
  download(suggestedName, await text(), mime);
  return `${suggestedName}, in this browser's downloads folder`;
}

export function exportGcode({ suggestedName }: { suggestedName: string }) {
  return save(suggestedName, { description: "G-code", accept: { "text/plain": [".gcode", ".nc"] } }, () =>
    engine<string>("gcode_text"),
  );
}

export function exportSvg({ suggestedName, ...traced }: { suggestedName: string; settings: unknown; widthMm: number; heightMm: number }) {
  return save(suggestedName, { description: "SVG drawing", accept: { "image/svg+xml": [".svg"] } }, () =>
    engine<string>("svg_text", traced),
  );
}

/** Asks for a file with an ordinary file input, for browsers without a file picker of the newer kind. */
function chooseFile(accept: string): Promise<File | null> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
}

/** Asks for an SVG drawing and reads its lines. */
export async function openDrawing() {
  let file: File | null;
  if (pickers.showOpenFilePicker) {
    try {
      const [handle] = await pickers.showOpenFilePicker({
        types: [{ description: "SVG drawings", accept: { "image/svg+xml": [".svg"] } }],
      });
      file = await handle.getFile();
    } catch (e) {
      if (cancelled(e)) return null;
      throw e instanceof Error ? e.message : String(e);
    }
  } else {
    file = await chooseFile(".svg,image/svg+xml");
  }
  if (!file) return null;
  const name = file.name.replace(/\.[^.]+$/, "");
  const sheet = await engine<{ parts: string[] }>("read_svg", { svg: await file.text(), name });
  return { name: sheet.parts[0] ?? name, sheet };
}