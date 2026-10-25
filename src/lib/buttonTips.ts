/**
 * Tooltips for buttons, by the text they show. Labelled settings explain
 * themselves through `hints.ts`; this covers the buttons, so that everything
 * clickable says what it does when the pointer rests on it.
 */
const BY_TEXT: Record<string, string> = {
  // Workflow rail
  Device: "Choose the machine you are making this job for",
  Material: "Say what you are working on and how big the piece is",
  Design: "Put a picture, drawing, words or a box on the material, and arrange them",
  Trace: "Turn the picture into outlines, and adjust its brightness, contrast and blur",
  Operations: "Choose the material, place the artwork, and decide what the machine does",
  Machine: "Connect to the machine, move it, and run the job",

  // Title bar
  Toolbox: "Your router bits and their dimensions",
  Assistant: "Ask questions about your project, or ask for changes to be made",
  Done: "Go back to the conversation",

  // Import and trace
  "Choose image…": "Pick a PNG, JPEG, WebP, BMP or GIF to trace, or an SVG to use as drawn",
  "Replace image…": "Pick a different picture or SVG; the current one is dropped",
  "Remove image": "Clear the picture and start again",
  Outline: "Trace the dark areas as solid shapes, for cutting or one-colour engraving",
  Colour: "Trace each colour as its own set of shapes",
  "Export SVG…": "Save the traced outlines as an SVG file",

  // Operations
  "Crop…": "Keep only part of the picture",
  "Cancel crop": "Leave the picture as it is",
  "Remove background…": "Keep the picture's subject and take away what is behind it; you can correct it by drawing on the picture",
  "Restore the picture": "Bring back the picture as it was imported, undoing any crop and background removal",
  Kept: "What you draw on the picture is kept",
  Removed: "What you draw on the picture is removed",
  "A loop round it": "Drag a line round an area; when you let go, everything inside it is kept or removed",
  "A brush": "Drag over an area to keep or remove it, like painting",
  "Marked up": "Show the whole picture, with blue over what will be removed",
  Finished: "Show the picture as it will be used, without its background",
  "Apply crop": "Keep what is inside the frame",
  Cancel: "Close without changing anything",
  "Add operation": "Add another operation to the job",
  "Estimate depth": "Work out how near each part of the photo is, for carving it as a shape",
  "Generate G-code": "Work out the tool's moves for the included operations",
  "Export G-code…": "Save the generated program as a file to send to the machine",
  "Open toolbox": "See and edit your router bits",
  Continue: "Go on to the next step",
  "Stay here": "Keep the generated job and stay on this step",
  "Change the box…": "Open the box maker with this box's sizes, to alter them",
  "Change the layers…": "Open the layers maker with these settings, to alter them",
  "Back to the picture": "Drop the layers and work from the picture again",
  "Change the text…": "Open the text maker with these words, to alter them",
  "Go back": "Clear the generated job and go back",

  Simple: "Choose what you want made and let the settings be picked for you",
  Advanced: "Show every setting",
  Draft: "Quickest and coarsest, for a trial run",
  Standard: "A balance of time and detail",
  Fine: "The most detail, and the longest job",
  "1. Main part": "Make the main part now",
  "2. Cut-out": "Make the cut-out now, after changing the bit",

  // Playback
  Paths: "Show the tool's path as lines",
  Result: "Show what the material will look like",
  "3D": "Show the material in 3D; drag to turn it, scroll to zoom",

  // Machine
  Connect: "Open the serial connection to the machine",
  Disconnect: "Close the connection to the machine",
  Home: "Send the machine to its home switches to find its position",
  Unlock:
    "Clear an alarm lock so the machine will move again. Only if you know where the head is: the machine will not stop at the edges until it is homed",
  Hold: "Move only while you hold a jog button or an arrow key down; let go to stop",
  Slow: "Move slowly, for lining up on a mark",
  Normal: "Move at an everyday speed",
  Fast: "Move quickly, for crossing the bed",
  "Go to XY zero":
    "Move the head back to the work zero (X0 Y0) at its current height, to check where the job will start. On a router, raise the bit clear of the material and clamps first",
  "Zero XY here": "Make the bit's current position X0 Y0 for the job",
  "Zero Z here": "Make the bit's current height Z0: touch it to the top of the material first",
  "Frame the job outline": "Move around the edge of the job without cutting, to check it fits",
  "Start job": "Begin sending the program; click again to confirm",
  Pause: "Stop moving, keeping the place in the job",
  Resume: "Carry on from where the job was paused",
  Stop: "Halt the machine and abandon the job",
  "Stop machine": "Halt the machine and abandon the job",
  "Stop the job here": "End the job now, at the bit change; Start then needs the job again from the beginning",
  "Test the probe": "Watch for the touch plate: touch it to the bit and the light should turn on",
  "Probe Z with the touch plate":
    "Lower the bit slowly onto the plate and set Z from it; click again to confirm. The tip must be within 10 mm of the plate",
  "Use the current position": "Take the machine's position from home as the place to change bits",

  // Saving
  "Save as one file with tool changes (M6)…":
    "Save the whole job as one file with M6 at each bit change, for senders that stop and let you set Z again",

  // Toolbox
  "Add a bit": "Add a new bit to the toolbox",
  "Read page": "Fetch this product page and fill in the bit's details from it",
  "Read pasted details": "Fill in the bit's details from the text pasted above",
  "Remove this bit": "Delete this bit from the toolbox",
};

function tip(button: HTMLButtonElement) {
  if (button.title) return;
  const text = button.getAttribute("aria-label") ?? button.innerText.trim();
  const known = BY_TEXT[text];
  // An icon button's accessible name already says what it does.
  const title = known ?? button.getAttribute("aria-label");
  if (title) button.title = title;
}

/** Gives every button a tooltip, now and as the interface changes. */
export function installButtonTips() {
  const apply = (root: ParentNode) => root.querySelectorAll("button").forEach(tip);
  apply(document);
  new MutationObserver((changes) => {
    for (const change of changes) {
      for (const node of change.addedNodes) {
        if (node instanceof HTMLButtonElement) tip(node);
        else if (node instanceof HTMLElement) apply(node);
      }
    }
  }).observe(document.body, { childList: true, subtree: true });
}
