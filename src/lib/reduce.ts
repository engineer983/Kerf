/**
 * Showing a large canvas small without spoiling it.
 *
 * An engraved photograph is made of dots far finer than a screen's pixels.
 * Shrunk carelessly, only a few of the dots land on each pixel and the picture
 * turns to noise and stripes that are not in the job at all. Halving it step
 * by step averages every dot into the pixel it falls on, which is what the eye
 * does with the real thing at arm's length.
 */

/** Canvases reused between calls, by size, since this runs on every frame of a playback. */
const spare = new Map<string, HTMLCanvasElement>();

function scratch(width: number, height: number): HTMLCanvasElement {
  const key = `${width}x${height}`;
  let canvas = spare.get(key);
  if (!canvas) {
    if (spare.size > 12) spare.clear();
    canvas = Object.assign(document.createElement("canvas"), { width, height });
    spare.set(key, canvas);
  }
  return canvas;
}

/**
 * Draws `source` into `target`, halved as many times as leaves it at least
 * `wanted` pixels wide: the width it will be shown at. `target` is resized to
 * suit, and holds the source unchanged when that is already small enough.
 */
export function reduceInto(target: HTMLCanvasElement, source: HTMLCanvasElement, wanted: number) {
  let halvings = 0;
  while (source.width / 2 ** (halvings + 1) >= wanted && source.width / 2 ** (halvings + 1) >= 2) halvings++;
  const sizeAt = (n: number) => [Math.max(1, Math.round(source.width / 2 ** n)), Math.max(1, Math.round(source.height / 2 ** n))];
  const [width, height] = sizeAt(halvings);
  if (target.width !== width || target.height !== height) {
    target.width = width;
    target.height = height;
  }
  let from = source;
  for (let n = 1; n <= halvings; n++) {
    const [w, h] = sizeAt(n);
    const to = n === halvings ? target : scratch(w, h);
    const context = to.getContext("2d")!;
    context.imageSmoothingEnabled = true;
    context.globalCompositeOperation = "copy";
    context.drawImage(from, 0, 0, w, h);
    from = to;
  }
  if (halvings === 0) {
    const context = target.getContext("2d")!;
    context.globalCompositeOperation = "copy";
    context.drawImage(source, 0, 0);
  }
}