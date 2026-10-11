/** The animated diagrams a hint can show; drawn in `components/Demos.tsx`. */
export type DemoId =
  | "stepover"
  | "depthPerPass"
  | "cutSide"
  | "tabs"
  | "offset"
  | "lineInterval"
  | "shading"
  | "threshold"
  | "safeHeight"
  | "raisedAreas"
  | "proportions"
  | "cornerRadius"
  | "taper"
  | "plunge"
  | "blur"
  | "passes"
  | "heightFrom";

export interface HintContent {
  text: string;
  demo?: DemoId;
}

/**
 * What each setting does, by the label it is shown under. A control looks its
 * hint up here unless it is given one directly, so every labelled control gets
 * a tooltip without each panel having to supply it.
 */
const BY_LABEL: Record<string, HintContent> = {
  // Trace
  "Automatic threshold": { text: "Picks the dividing line between black and white for you, from the image's own mix of light and dark.", demo: "threshold" },
  Threshold: { text: "Pixels darker than this become black and are traced; lighter ones are ignored. Raise it to pick up more of the image.", demo: "threshold" },
  Brightness: { text: "Lightens or darkens the image before it is used. For engraving and carving this shifts the whole picture lighter or deeper." },
  Contrast: { text: "Pushes light and dark further apart, or pulls them together. More contrast gives a bolder engraving or a deeper-looking carving." },
  Blur: { text: "Softens the image. It removes speckle from a trace and smooths the surface of a carving.", demo: "blur" },
  Sharpen: {
    text: "Brings out edges and fine detail such as eyes, hair and fabric. An engraved photograph comes out softer than the picture, and this makes up for it: 60 to 100% suits most photos. Too much puts a pale halo round dark edges and makes skin look rough. It is left out of carvings and stacked layers, where the halo would be cut as a ridge.",
  },
  "Invert (trace light areas)": { text: "Swaps black and white, so the light parts of the image are the ones traced." },
  "Colour detail": { text: "How finely colours are told apart. Higher keeps more separate colours; lower merges similar ones into fewer shapes." },
  "Layer separation": { text: "How different two colours must be before they become separate shapes. Higher gives fewer, larger shapes." },
  "Ignore specks under": { text: "Shapes smaller than this many pixels across are dropped, which removes dust and noise from the trace." },
  "Smooth curves": { text: "Fits smooth curves through the outline. Turn it off to keep straight-edged shapes exactly as drawn." },
  "Corner sharpness": { text: "How tight a turn must be to stay a sharp corner instead of being rounded into a curve." },
  "Show original image": { text: "Shows the imported picture faintly behind the trace, so you can compare them." },

  // Device
  "Bed width": { text: "The machine's usable travel from left to right." },
  "Bed depth": { text: "The machine's usable travel from front to back." },
  "X0 Y0 corner": { text: "The corner the machine calls X0 Y0. Most routers use the front left; some lasers home to the back left. If this is wrong, jobs come out mirrored front to back." },
  "Full power (S)": { text: "The S value in the G-code that means 100% laser power. It is GRBL's $30 setting, usually 1000." },
  "Router speed": { text: "How fast the bit spins. Cutting speeds are worked out from it. On a trim router this is the dial setting, not something the G-code controls." },

  // Material and artwork
  Thickness: { text: "How thick the material is. The preview uses it, and a cutout goes this deep by default." },
  Width: { text: "The size from left to right, in millimetres." },
  Length: { text: "The material's size from front to back." },
  Height: { text: "The artwork's size from back to front." },
  "Keep proportions": { text: "When on, changing the width changes the height with it, so the picture is never stretched.", demo: "proportions" },
  "From left": { text: "How far the artwork's left edge is from the left edge of the bed." },
  "From front": { text: "How far the artwork's front edge is from the front edge of the bed." },
  "From back": { text: "How far the artwork's back edge is from the back edge of the bed." },

  // Operations
  Bit: { text: "The router bit used for this operation, from your toolbox. Choosing one sets the speeds below to suit it and the material." },
  "Applies to": { text: "Run this operation on every traced shape, or only on shapes of one colour." },
  Speed: { text: "How fast the tool moves while cutting or engraving, in millimetres per minute. Slower cuts deeper or burns darker." },
  Stepover: { text: "The gap between one row of a carving and the next. Smaller gives a smoother surface and a much longer job.", demo: "stepover" },
  "Raised areas": { text: "Which parts of the picture stand proud. The other parts are cut deeper.", demo: "raisedAreas" },
  Depth: { text: "How deep the deepest part of the cut is, measured down from the top of the material." },
  "Depth per pass": { text: "How much deeper the bit goes on each trip. Several shallow passes are easier on the bit than one deep one.", demo: "depthPerPass" },
  "Plunge speed": { text: "How fast the bit moves straight down into the material. Bits cut poorly downwards, so this is slower than the cutting speed.", demo: "plunge" },
  "Safe height": { text: "How high the bit lifts above the material before moving to a new place, so it clears clamps and the work.", demo: "safeHeight" },
  Power: { text: "Laser power as a percentage of full. More power burns darker and deeper." },
  Passes: { text: "How many times the operation is repeated over the same path. More passes cut deeper without more power.", demo: "passes" },
  "Line interval": { text: "The gap between one engraved line and the next. Smaller is finer and darker, and takes longer.", demo: "lineInterval" },
  Shading: { text: "How greys are made. Dots uses full-power dots, more of them where it is darker; Varying power changes the laser's strength instead.", demo: "shading" },
  "Height from": { text: "What decides how deep each part is carved: how bright the picture is there, or how near the camera a model estimates it to be.", demo: "heightFrom" },
  "Picture detail": { text: "How strongly the picture's own linework, such as feathers, hair, fabric and lettering, is cut into the estimated shape. The depth model only finds the broad form; this puts the drawing back." },
  "Flatten background": { text: "Levels off the furthest part of the scene into a flat background, and spreads the subject over the full depth so it stands out more." },

  // Cutout
  Offset: { text: "How far outside the artwork the cut line is. A negative number puts it inside.", demo: "offset" },
  "Cut side": { text: "Where the bit runs relative to the line. Outside keeps the piece the size of the line; inside makes a hole that size; on splits the difference.", demo: "cutSide" },
  Shape: { text: "The shape of the cut line: a rectangle around the artwork, or the oval that fits it." },
  "Corner radius": { text: "Rounds the corners of the cut line by this radius. Zero leaves them square.", demo: "cornerRadius" },
  Tabs: { text: "Small bridges left uncut so the piece stays attached until you free it by hand. Without them it can come loose and be damaged.", demo: "tabs" },
  "Tab width": { text: "How long each bridge of material is, along the cut.", demo: "tabs" },
  "Tab height": { text: "How thick each bridge is, up from the bottom of the cut.", demo: "tabs" },
  "Include in the job": { text: "Untick to leave this operation out of the G-code. Use it to run operations one at a time, such as when they need different bits." },

  // Toolbox
  Flutes: { text: "The number of cutting edges on the bit. It is used to work out the cutting speed." },
  "Cutting diameter": { text: "How wide the bit cuts. For a V-bit, its widest point." },
  "Tip diameter": { text: "The diameter of the small ball at the tip: twice its radius. A bit marked R0.25 has a 0.5 mm tip.", demo: "taper" },
  "Cutting length": { text: "How far up the bit its cutting edges reach, which is the deepest it can cut." },
  Shank: { text: "The diameter of the plain part held in the collet." },
  "Taper per side": { text: "The angle of each side of a tapered bit, measured from its centre line.", demo: "taper" },
  Angle: { text: "The full angle of the V, from one side to the other." },
  Flute: { text: "The number of cutting edges on the bit." },
};

export function hintFor(label: string): HintContent | undefined {
  return BY_LABEL[label];
}
