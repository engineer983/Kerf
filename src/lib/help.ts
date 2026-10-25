import type { MachineKind } from "./ipc";

// The app's own guide: how to do things in Kerf, and good practice at the
// machine. Written to be read at the bench, so it is short and in order.
//
// What it says about machines and materials is general practice for hobby
// diode lasers and belt-driven routers. Speeds and powers are left to tests
// on scrap, because they depend on the particular machine and material.

export interface HelpBlock {
  heading?: string;
  text?: string;
  /** Things done in order. */
  steps?: string[];
  /** Things to know, in no order. */
  points?: string[];
  /** Something that can hurt a person, a machine or a piece of work. */
  warning?: string;
}

export interface HelpTopic {
  id: string;
  title: string;
  group: "Getting started" | "Laser" | "Router" | "How to make" | "When something goes wrong";
  /** The machine it is about, for topics that only apply to one kind. */
  machine?: MachineKind;
  blocks: HelpBlock[];
}

export const HELP_GROUPS: HelpTopic["group"][] = ["Getting started", "Laser", "Router", "How to make", "When something goes wrong"];

export const HELP_TOPICS: HelpTopic[] = [
  {
    id: "steps",
    title: "The six steps",
    group: "Getting started",
    blocks: [
      { text: "Every job goes through the same numbered steps, down the left side of the window. You can go back to any of them at any time." },
      {
        steps: [
          "Device: choose the machine. Everything after is tailored to it: a laser is offered engraving and cutting, a router carving and profiles.",
          "Material: say what you are working on, how thick it is, and how big the piece is. The design is laid out on it at real size, so measure the piece rather than guessing.",
          "Design: put something on the material. Start with a picture, a drawing, words or a box, then add more words and drawings and arrange them.",
          "Trace: how a picture is turned into shapes, and its brightness, contrast and blur. In Simple mode you can skip it.",
          "Operations: what the machine does. Simple asks what you want made and sets everything up; Advanced shows every setting. Generate G-code here and check the preview.",
          "Machine: for a router Kerf can drive directly, connect, set zero and run the job. A laser that only takes files has no step 6: you export the G-code instead.",
        ],
      },
      {
        heading: "Before your first real job",
        warning:
          "Kerf is new, and its G-code has not yet been proven on your machines. Make the first job on each machine a small one on scrap, at low power or as an air cut, with your hand near the stop. Check it comes out the right size, the right way round, and where the preview showed it.",
      },
      {
        heading: "Things worth knowing",
        points: [
          "Hover over any setting's name for an explanation, often with a small animation.",
          "The panels on the right can be made wider or narrower: drag a panel's left edge. Double-click the edge to go back to its usual width.",
          "Ctrl+Z undoes a change to the settings; the arrows in the title bar do the same.",
          "Click the canvas, then the arrow keys move whatever is selected 1 mm at a time, or 10 mm with Shift. They only move the design on screen, never the machine.",
          "Centre everything on the material, in step 3, moves the artwork and its words and drawings together into the middle of the material.",
          "The Result view shows roughly what the finished piece will look like, and 3D lets you turn it. Press play to watch the job run.",
          "A project is saved to Your projects each time you generate, and when you press Save.",
          "The assistant can set a job up from a wish such as \"make this as realistic as possible\". It cannot move or start a machine.",
        ],
      },
    ],
  },
  {
    id: "material",
    title: "Measuring your material",
    group: "Getting started",
    blocks: [
      { text: "Step 2 asks for the piece's width, length and thickness. They matter more than they seem to." },
      {
        points: [
          "Width and length decide how big the design can be and where it sits. The piece starts in the machine's origin corner: front-left on the X-Carve, back-left on the WeCreat.",
          "If the piece is somewhere else on the bed, type where it sits under Where it sits: how far its left edge is from the bed's left edge, and how far its front (or back) edge is from the bed's. The design moves with it, so it stays where it was on the piece. Put it back in the corner undoes that. With a camera, Find the material does it for you.",
          "Kerf measures every job from the bed's origin corner, not from the material. If you set the work zero on the material's own corner (Zero XY here), leave the material in the corner in step 2; otherwise the job lands that far off the material. Frame the job first to check.",
          "Thickness decides how deep a cut-out goes and how deep a box's joints are cut. Sheet sold as 3 mm is often 2.7 to 3.2 mm, so measure it with calipers, or use the thickness the WeCreat reports when it auto-focuses.",
          "On a router, the material type sets the cutting speeds. On a laser it only changes how the preview looks.",
          "Leave a margin. Clamps, tape and the machine's own reach all need room at the edges.",
        ],
      },
    ],
  },
  {
    id: "camera",
    title: "A camera over the bed",
    group: "Getting started",
    blocks: [
      {
        text: "A camera pointed down at the bed (a USB webcam is fine) lets you see the design on top of what's really on the bed. Kerf opens the camera only when you take a picture, closes it straight after, and never saves or sends the picture anywhere.",
      },
      {
        heading: "Setting it up",
        steps: [
          "Fix the camera so it can't move. Every time it moves, it needs lining up again.",
          "In step 1, Device, find the Camera section, press Find cameras, and choose yours. The first time, the computer asks whether Kerf may use the camera.",
          "Take a picture with the bed clear, then press Line up the camera.",
          "Drag each numbered marker onto the corner it names. If a corner can't be seen, put the marker on another place you can measure and type where it is.",
          "Leave Check ticked: the blue grid should sit on the real bed, the red arrow should point along X and the green one along Y. If Kerf says the picture looks mirrored, a marker is on the wrong corner.",
          "Press Use this lining up. From then on, the button in the canvas's bottom-right corner takes a new picture whenever you like.",
        ],
      },
      {
        heading: "How close it is",
        points: [
          "The picture is lined up at the bed's surface. The top of thick material sits nearer the camera, so it looks larger and shifted outwards. The shift is roughly the distance from the spot under the camera, times the thickness, divided by the camera's height. With the camera 400 mm up, a point 200 mm out on 18 mm stock is about 9 mm off.",
          "Wide-angle lenses curve straight lines near the edges of the picture. Kerf doesn't straighten that yet, so the corners you lined up on are the most accurate places.",
          "If the bed size changes, or you use another camera, Kerf asks for lining up again. Changing the X0 Y0 corner doesn't move the picture.",
          "If the camera gets knocked or moved, line it up again. Kerf can't tell. The canvas shows when it was last lined up.",
          "The orange outline is where Kerf thinks the material is. If the real sheet in the picture is somewhere else, press Find the material, type where it sits in step 2, or move the sheet, before cutting.",
          "After a job runs, or the material is moved or resized by hand, the picture is marked as maybe out of date. Take a new one.",
        ],
      },
      {
        heading: "Before cutting",
        warning:
          "The picture shows the bed's corner as X0 Y0, but a job runs from the machine's work zero. If you've moved the zero (Zero XY here, or G10 or G92), the job won't go where the picture shows it, and Kerf says so on the canvas. Kerf can't see a zero moved by other software, or on a machine without homing switches, where 0,0 is wherever the head was when it was switched on. Either way, trace the job with Frame first, with the laser off, and check it stays on the material and clear of clamps.",
      },
      {
        heading: "If the camera doesn't work",
        points: [
          "Windows: check Settings > Privacy & security > Camera, including \"Let desktop apps access your camera\". Close any other program using the camera.",
          "Mac: the camera needs macOS 12 or later. Check System Settings > Privacy & Security > Camera, and allow Kerf.",
          "Linux: your user must be in the video group. The deb needs the gstreamer1.0-plugins-good package; the AppImage brings its own.",
          "In a browser, the page must be https or localhost, and the browser asks before letting it use the camera.",
        ],
      },
    ],
  },

  {
    id: "find-material",
    title: "Finding the material with the camera",
    group: "Getting started",
    blocks: [
      {
        text: "Once the camera is lined up, Kerf can look for your piece of material in the picture and put the design in the middle of it. It looks for something plain, such as plywood, MDF, card or masked acrylic, sitting on something busy, such as honeycomb, slats or clamps.",
      },
      {
        steps: [
          "Put the material on the bed and press Find the material, in the canvas's bottom-right corner or in step 2. If the picture may be out of date, Kerf takes a new one first.",
          "Kerf draws the outline it found over the picture, with what it found and anything it isn't sure of. Nothing has changed yet.",
          "If the outline is wrong, drag its corners onto the real corners. If Kerf found the wrong thing, or nothing, press Click on the material to try again and click on a plain part of the piece.",
          "If the size is within 10 mm of what you typed, Kerf keeps your size, because a ruler beats a camera. Untick Keep my size to use the camera's.",
          "Press Use this outline. The material moves there and the design is centred on it. One Ctrl+Z puts it all back.",
          "Then adjust by dragging, typing, or with the arrow keys, and frame the job before cutting.",
        ],
      },
      {
        heading: "What to know",
        points: [
          "Kerf can't turn a design. If the material is at an angle, Kerf uses the straight part in the middle of it and says how big that is. Straighten the material for the full size.",
          "It looks for a rectangle. A round piece is found by the square around it, so its middle is right. For an L-shaped or ragged offcut, check the outline and the design's place yourself.",
          "Thick material looks shifted in the picture, more so towards the edges, because its top is nearer the camera. Check the far edges.",
          "Clear and mirrored acrylic show the bed or the room, so they are usually not found. Click on them, or drag the corners into place.",
          "The design's size isn't changed. If it doesn't fit, Make it fit shrinks everything together to fit, leaving 10 mm round it.",
          "The place found is measured from the bed's origin corner, so the job's work zero must be that corner too. Don't set the zero on the material's corner when the material has been found or moved.",
        ],
      },
      {
        heading: "Before cutting",
        warning:
          "The outline is a guess. A wrong one puts a correct job in the wrong place: on a laser, onto the honeycomb or slats, which is a fire risk; on a router, into a clamp. Check the outline doesn't include a clamp, keep the design at least 10 mm from any clamp, and frame the job with the laser off before you start.",
      },
    ],
  },

  // ---- Laser ---------------------------------------------------------------
  {
    id: "laser-safety",
    title: "Staying safe with the laser",
    group: "Laser",
    machine: "laser",
    blocks: [
      {
        warning: "Never leave a laser running on its own. Wood, card and acrylic catch fire, and a small flame becomes a big one in seconds. Stay where you can see it, with a fire blanket or extinguisher within reach.",
      },
      {
        points: [
          "Keep the lid shut while it runs. The enclosure is what protects your eyes. If you ever run with it open, wear laser glasses rated for a blue diode laser.",
          "Vent the fumes outdoors or through the purifier. Smoke from wood and acrylic irritates lungs and eyes, and it clouds the lens.",
          "Clear offcuts and dust out of the bed before each job. Small scraps are what catch light first.",
          "If something flares, pause or stop the job and open the lid only once the beam is off.",
        ],
      },
      {
        heading: "Never put these in a laser",
        points: [
          "PVC, vinyl and imitation leather: they give off hydrogen chloride, which harms you and corrodes the machine.",
          "ABS and polycarbonate: they melt, burn dirty and give off toxic fumes.",
          "Fibreglass and carbon fibre: the resin fumes are harmful.",
          "Foam of any kind: it catches fire.",
          "Anything you cannot identify. If you do not know what a plastic is, do not burn it.",
        ],
      },
    ],
  },
  {
    id: "laser-practice",
    title: "Getting good results from the laser",
    group: "Laser",
    machine: "laser",
    blocks: [
      {
        heading: "Every job",
        points: [
          "Focus on the piece you are about to use. Run the machine's auto-focus whenever the material changes; a millimetre out of focus makes lines wide and weak.",
          "Keep the material flat. Warped plywood is in focus in one place and out of it in another, so pin or weigh it down.",
          "Test on an offcut of the same material first. Start with low power, raise it until the mark is as dark or the cut as deep as you want, and write the numbers down for next time.",
          "Clean the lens and its window when results start to fade.",
        ],
      },
      {
        heading: "Engraving",
        points: [
          "Light, even woods engrave best: basswood, birch plywood, maple. Pine's hard and soft grain burns unevenly.",
          "For photographs, choose Engrave the picture, and try Optimise picture. Dots (dithering) suit wood, which does not darken evenly with power.",
          "A line interval near 0.1 mm is about as fine as the beam allows. Wider is faster and lighter.",
          "Masking tape over the surface keeps smoke stains off it. Peel it after the job.",
          "Bold shapes (fill) suits logos and lettering; Outline only follows their edges with a single line and is much quicker.",
        ],
      },
      {
        heading: "Cutting",
        points: [
          "Lift the material off the bed, on a honeycomb or pins, so smoke can escape underneath and the back does not scorch.",
          "Use air assist for cutting. It blows the smoke out of the cut, which gives cleaner edges and deeper cuts.",
          "Several quick passes usually cut cleaner than one slow one, with less charring.",
          "Cuts run last in a Kerf job, so the piece is still held in the sheet while it is engraved.",
          "Small parts drop through the bed. Leave them attached, or put a tray under them.",
        ],
      },
      {
        heading: "What a blue diode laser can and cannot do",
        points: [
          "It engraves and cuts wood, plywood, MDF, card, leather and dark, opaque acrylic.",
          "It cannot cut clear acrylic or engrave clear glass: the light passes straight through.",
          "It cannot cut metal. It can mark metal with a coating, such as anodised aluminium or paint, by removing the coating.",
        ],
      },
    ],
  },
  {
    id: "laser-sending",
    title: "Sending a job to the WeCreat",
    group: "Laser",
    machine: "laser",
    blocks: [
      {
        text: "Kerf does not connect to the WeCreat. It writes a G-code file, with the commands the machine expects at the start and end, and you run that file from LightBurn.",
      },
      {
        steps: [
          "In Kerf, finish the job in step 5 and press Generate G-code, then Export G-code… and save the file.",
          "Put the material in the back-left corner of the bed and close the lid.",
          "Run the machine's auto-focus on the material.",
          "In LightBurn, connect to the laser, then in the Laser window choose Run GCode and pick the file Kerf saved.",
          "Stay with it until it finishes.",
        ],
      },
      {
        heading: "The first time",
        warning:
          "Which way the WeCreat counts front-to-back was worked out from its settings, not proven by a burn. Before any real job, engrave something small and lopsided, such as the letter F, at low power on scrap. Check it is not mirrored or upside down and that it lands where Kerf's preview showed it. If it does not, tell the assistant or change the X0 Y0 corner in step 1.",
      },
      {
        points: [
          "Kerf's files never move the laser up or down, so focusing is always yours to do first.",
          "Power in Kerf is a percentage of full power. Speed is in millimetres a minute.",
          "The thickness the machine measures when it focuses is the number to enter in step 2 for cut-outs and boxes.",
        ],
      },
    ],
  },

  // ---- Router --------------------------------------------------------------
  {
    id: "router-safety",
    title: "Staying safe with the router",
    group: "Router",
    machine: "cnc",
    blocks: [
      {
        warning: "A spinning bit does not stop for fingers, sleeves or hair. Keep your hands out of the machine while the spindle turns, and never reach in to clear chips.",
      },
      {
        points: [
          "Wear safety glasses and hearing protection every time. Bits break, and routers are loud enough to damage hearing.",
          "Collect the dust or wear a dust mask. Fine wood dust is harmful to breathe, MDF dust especially.",
          "No gloves, loose sleeves, dangling cords or untied long hair near the spindle.",
          "Stay with the machine while it runs. Know where the stop is: Stop machine in Kerf, and the power switch on the machine itself.",
          "Unplug or switch off the router before changing a bit.",
        ],
      },
    ],
  },
  {
    id: "router-setup",
    title: "Setting up a job on the router",
    group: "Router",
    machine: "cnc",
    blocks: [
      {
        heading: "Hold the material down",
        points: [
          "It must not move at all. Clamp it at several points, well clear of where the bit will go.",
          "For thin sheet, and for parts that are cut right through, painter's tape on the bed and on the material with superglue between them holds without clamps in the way.",
          "Put a sacrificial board underneath whenever a cut goes right through.",
          "Use Frame the job outline in step 6 to run round the job's edge above the material, and check nothing is in the way.",
        ],
      },
      {
        heading: "Set zero",
        steps: [
          "Home the machine if it has homing switches, so it knows where it is.",
          "Jog the bit to the front-left corner of the material and press Zero XY here. Kerf measures everything from that corner on the X-Carve.",
          "Choose Hold to move the bit while you hold the button or an arrow key; let go to stop. Click the jog pad first for the keys to work.",
          "Lower the bit until it just touches the top of the material. A slip of paper under the bit should drag but still slide. Press Zero Z here.",
          "Raise the bit clear before starting.",
        ],
      },
      {
        heading: "Changing the bit",
        text: "A job can use more than one bit, such as a flat bit to rough out a relief, a tapered ball nose to finish it and a flat bit to cut it out. Step 5 lists the bits in order and how many times the job stops. When Kerf runs the job, it stops at each change with the spindle off and the bit raised over the work zero. Fit the next bit, set Z again (with the touch plate, or lower the bit onto the material and press Zero Z here), then press Continue twice. Kerf won't carry on until Z has been set again, and it won't let you move the X/Y zero while it waits. If you use another sender, save one file per bit and set Z before running each one.",
      },
      {
        heading: "Setting Z with a touch plate",
        steps: [
          "Measure the plate's thickness with calipers and enter it in step 1, under Changing bits. Kerf leaves it blank until you do.",
          "Clip the plate's wire to the bit, press Test the probe in step 6, then touch the plate to the bit. The light should turn on. Kerf won't probe until it has seen this once.",
          "Lay the plate on the material under the bit, and jog the tip to within about 10 mm of it.",
          "Press Probe Z with the touch plate, then again to confirm. The bit comes down slowly, touches twice, sets Z and lifts 5 mm.",
          "Take the clip off the bit and move the plate away before carrying on. At a bit change Kerf asks you to tick that you have.",
        ],
      },
      {
        warning: "If the plate isn't under the bit, nothing stops it except the 10 mm limit: it goes down into whatever is below. Check the plate is there before you probe.",
      },
      {
        heading: "Router speed",
        text: "Kerf works its feeds out from the router's speed, set in step 1. It starts at 16,000 rpm, which is dial 1 on the DeWalt 611 fitted to many X-Carves. If your dial is set differently, enter the speed it gives.",
      },
    ],
  },
  {
    id: "router-practice",
    title: "Getting good results from the router",
    group: "Router",
    machine: "cnc",
    blocks: [
      {
        heading: "Bits",
        points: [
          "Use the bit Kerf names for the job, since the speeds were worked out for it. Add your own bits in the Toolbox.",
          "A flat end mill follows lines and cuts pieces out. A ball nose carves reliefs. A tapered ball nose holds fine detail and is stiffer than a thin straight bit.",
          "Push the bit well into a clean collet and tighten it properly. Let as little of it stick out as the job allows: a long bit bends and chatters.",
          "An up-cut bit clears chips well but can fuzz the top edge. A down-cut bit leaves a clean top edge.",
        ],
      },
      {
        heading: "Speeds and depth",
        points: [
          "Kerf's speeds are starting points worked out from the bit, the material and the router's speed. Try them on scrap before trusting them.",
          "Too fast or too deep: the machine chatters, the bit bends or snaps, or the carriage stalls and the job loses its place.",
          "Too slow: the wood burns and the bit makes dust instead of chips.",
          "As a rule of thumb on a belt-driven machine like the X-Carve, cut no deeper in one pass than half the bit's diameter in wood. Several light passes beat one heavy one.",
        ],
      },
      {
        heading: "Carving a relief",
        points: [
          "Reliefs take hours. Run a Draft first to see whether the shape is what you want, then Standard or Fine.",
          "Close-grained woods such as maple, cherry and walnut hold detail. Pine tears. MDF carves cleanly but needs sealing.",
          "For a photograph, take the height from estimated depth, and keep Picture detail low so skin and sky stay smooth.",
          "Leave a few millimetres of material under the deepest cut, and stay within the bit's cutting length.",
          "A soft brass brush or light sanding lifts the fuzz a relief leaves behind.",
        ],
      },
      {
        heading: "Cutting a piece out",
        points: [
          "Run the cut-out last, after any carving, so the piece is held while it is worked on.",
          "Kerf keeps to this order however the operations are listed: engraving and carving first, then added text and drawings, then anything that cuts right through (such as a box's outlines), then the cut-out.",
          "Keep the tabs. They stop the piece coming loose and being thrown or gouged on the last pass. Cut them with a chisel or flush saw afterwards and sand them off.",
          "Set the depth to the material's real thickness, with scrap board underneath.",
        ],
      },
    ],
  },
  {
    id: "router-running",
    title: "Running a job from Kerf",
    group: "Router",
    machine: "cnc",
    blocks: [
      {
        warning: "Kerf's sender has only been tested against a simulated controller so far. For the first real job, run it as an air cut: set Zero Z a few centimetres above the material so the bit never touches it, and watch that the moves match the preview.",
      },
      {
        steps: [
          "Generate the G-code in step 5 and look it over in the Result and 3D views.",
          "In step 6, choose the machine's port and press Connect.",
          "Home the machine, or Unlock it if it starts in an alarm.",
          "Jog to the front-left corner of the material and set Zero XY, then touch the bit to the top and set Zero Z, or set Z with the touch plate (test it first, and take the clip off afterwards).",
          "Raise the bit clear, then press Go to XY zero to check where the origin is.",
          "Press Frame the job outline and check it clears the clamps and stays on the material.",
          "Put on glasses and ear protection, switch the router on, and press Start job, then again to confirm.",
          "Pause holds the machine where it is; Resume carries on. Stop halts it and abandons the job.",
          "If the job uses more than one bit, it stops at each change and waits. Change the bit, set Z again, take any touch plate clip off, and press Continue twice.",
        ],
      },
      {
        points: [
          "The console at the bottom shows what the controller says. An alarm means it stopped itself, usually at a limit switch.",
          "After a Stop, the machine may no longer know exactly where it is. Home it and set zero again before the next job.",
        ],
      },
    ],
  },

  // ---- How to make ---------------------------------------------------------
  {
    id: "make-photo-laser",
    title: "Engrave a photograph",
    group: "How to make",
    machine: "laser",
    blocks: [
      {
        steps: [
          "Step 1: choose the laser.",
          "Step 2: enter the material and the size of the piece.",
          "Step 3: press Choose image… and pick the photo, or paste one with Ctrl+V. Drag it or its handles to place and size it, or use Fill the material. Press Continue.",
          "Step 5, in Simple: choose Engrave the picture. Press Optimise picture to spread its tones over the full range.",
          "Set Power and Speed from your test on scrap.",
          "Press Generate G-code and look at the Result view. The split button beside Paths, Result and 3D compares it with your last attempt.",
          "To free the piece, tick Cut the piece out when done and set the cut's power, speed and passes.",
          "Export G-code… and run the file on the laser.",
        ],
      },
      {
        heading: "For a better engraving",
        points: [
          "Start with a sharp photo with one clear subject and a plain background.",
          "If the background is busy, take it away: Remove background… in step 3 keeps just the subject.",
          "A little extra contrast in step 4 usually helps. The canvas shows the picture in the greys the laser will read.",
          "Sharpen, in step 4, brings out eyes, hair and fabric, which an engraving otherwise softens. Optimise picture sets it to 80% for you; past about 120% skin starts to look rough.",
          "The Result view shows the engraving as it looks at arm's length. Zoom in with the mouse wheel to see the dots themselves.",
          "What the wood does matters as much as the picture: pale, even-grained wood such as basswood or birch plywood shows a photo best. Test power and speed on scrap of the same wood until the darkest parts are dark brown, not charred.",
          "Fine gives the most detail and takes longest. Try Draft on scrap first.",
        ],
      },
    ],
  },
  {
    id: "make-photo-router",
    title: "Carve a photograph as a relief",
    group: "How to make",
    machine: "cnc",
    blocks: [
      {
        steps: [
          "Step 1: choose the router. Check the router speed matches your dial.",
          "Step 2: enter the material, its real thickness and the size of the piece.",
          "Step 3: Choose image… and place the picture. Press Continue.",
          "Step 5, in Simple: choose 3D carving. Kerf works out the picture's shape, which takes a while the first time.",
          "Choose Draft, Standard or Fine, and the Depth. Deeper reads as more lifelike, but leave material underneath.",
          "Fit the bit named under Bit to fit.",
          "Generate G-code and turn the piece over in the 3D view. The time is shown under the button.",
          "Step 6: hold the material down, set zero, frame it, and run it.",
        ],
      },
      {
        heading: "For a better carving",
        points: [
          "In Advanced, Picture detail adds the photo's own linework back onto the shape. A little goes a long way: start near 10%.",
          "Flatten background levels off what is behind the subject, so the subject uses the whole depth.",
          "Remove background… in step 3 goes further and takes away what is behind the subject altogether.",
          "A bigger piece shows more detail with the same bit.",
        ],
      },
    ],
  },
  {
    id: "remove-background",
    title: "Remove a picture's background",
    group: "How to make",
    blocks: [
      {
        text: "This keeps the subject of a photograph and takes away what is behind it, so only the subject is engraved, traced or carved.",
      },
      {
        steps: [
          "Step 3, Design: select the picture and press Remove background….",
          "Wait while the subject is found. The first time, a small model is downloaded; after that it needs no connection. The picture never leaves your computer.",
          "Blue lies over what will be removed. Look for parts of the subject under the blue, and for background left clear.",
          "To bring a part back, choose Kept and A loop round it, then drag a line round that part and let go. Everything inside the loop is kept.",
          "To take a part away, choose Removed and do the same.",
          "For edges and small places, choose A brush and paint over them. Undo, or Ctrl+Z, takes back the last thing drawn.",
          "Choose Finished to see the picture as it will be used, then press Remove the background.",
        ],
      },
      {
        heading: "Good to know",
        points: [
          "The subject is found best in photographs of people, and of things that stand clear of what is behind them. Where the subject and the background are the same shade, such as a white shirt against a white wall, part of the subject is lost and has to be drawn back.",
          "To do it all by hand, press Remove everything and draw a loop round just what you want.",
          "Logos and drawings do not need this: tracing already leaves a white background alone.",
          "The background becomes white, which a laser leaves untouched. For a carving where dark is low, black gives a flat ground for the subject to stand up from.",
          "Restore the picture, in step 3, brings back the picture as it was imported, and undoes any crop too.",
          "A carving's shape is worked out again afterwards, since the picture has changed.",
        ],
      },
    ],
  },
  {
    id: "make-sign",
    title: "Make a sign: a picture with words",
    group: "How to make",
    blocks: [
      {
        steps: [
          "Step 2: enter the size of the sign's blank. Everything is fitted to it.",
          "Step 3: add the first thing, a picture or Text…, then press Add text… for each further line of words, or Add a drawing… for an SVG.",
          "Click anything on the canvas or in the list to select it. Drag it to move it, drag its handles to resize it, or use Centre it.",
          "To size something to a space, use Fit into a box: enter the space's width and height and press Fit to the box. Tick Stretch to fill it exactly to squeeze it to that shape.",
          "Press Continue. In step 5, the main artwork is set up as usual, and Added text and drawings has its own settings: on a laser, engraved solid or outlined, with its own power and speed; on a router, its own depth.",
          "Tick Cut the piece out when done to cut round everything together.",
          "Generate, check the preview, and make it.",
        ],
      },
      {
        heading: "Words on a curve, or round the picture",
        points: [
          "In the text maker, Layout chooses how the words run: Straight, On a curve, or Round the artwork.",
          "On a curve bends them round a circle: to the right they arch over, to the left they curve under.",
          "Round the artwork follows a circle, an oval or the picture's own frame, centred over its top or under its bottom, with a gap you set.",
          "Words set round the artwork are drawn to fit it as it is. If you resize the artwork, select the words and press Change the text… to fit them again.",
        ],
      },
    ],
  },
  {
    id: "make-text",
    title: "Lettering and name plates",
    group: "How to make",
    blocks: [
      {
        steps: [
          "Step 3: press Text…, type the words, and press Enter for a new line.",
          "Pick a font, a letter size and a layout. The preview shows the result.",
          "Press Make the text, then size and place it on the material.",
          "In step 5: on a laser, Bold shapes engraves the letters solid and Outline only traces round them. On a router, Follow the lines runs the bit round each letter.",
        ],
      },
      {
        points: [
          "Bold, simple fonts stay readable when small. Thin and script fonts lose their fine strokes.",
          "On a router the middle of the bit follows each letter's edge, so letters come out a little heavier than drawn. A fine or V-shaped bit suits small text.",
        ],
      },
    ],
  },
  {
    id: "make-box",
    title: "Make a box",
    group: "How to make",
    blocks: [
      {
        steps: [
          "Step 2: enter the material and, most of all, its measured thickness.",
          "Step 3: press Box maker… and give the outside width, depth and height. Tick With a lid for a closed box.",
          "On a laser, enter the Kerf, the width of the line the beam burns away. On a router, choose the flat bit you will cut with.",
          "Press Make the box. The panels are laid out to fit your material. If they need more than one piece, choose which sheet to cut in step 5.",
          "To put words on a panel, press Add text… in step 3 and move them onto it. They are engraved before the panels are cut free, so they land where you put them.",
          "Generate and cut each sheet.",
        ],
      },
      {
        heading: "Getting the joints to fit",
        points: [
          "Cut one pair of panels from an offcut and try the joint before cutting the lot.",
          "Laser: if the fingers are loose, raise the Kerf a little; if they will not go together, lower it. A change of 0.05 mm is enough to feel.",
          "The joints are cut exactly as deep as the thickness you gave. If the fingers stand proud or fall short, the thickness is the number to correct.",
          "Router: the cut is carried into each inside corner, which leaves a small notch there. That is what lets a square finger fit a round bit's corner.",
        ],
      },
      {
        warning: "On a router nothing holds a panel once it is cut free. Stick the sheet down with tape and superglue, over scrap board, so panels cannot shift into the bit.",
      },
    ],
  },
  {
    id: "make-layers",
    title: "Stacked layers",
    group: "How to make",
    blocks: [
      { text: "A picture cut as several pieces of thin sheet, glued one on another so its high parts stand proud, like a map's contour lines." },
      {
        steps: [
          "Put a picture on the material in step 3, select it, and press Stacked layers….",
          "Choose how many layers, counting the base, and the finished width.",
          "For a photograph, take the heights from Estimated depth. For a drawing or a height map, use Brightness.",
          "Press Make the layers. They are laid out on sheets to fit your material.",
          "Cut each sheet, then glue layer 2 onto the base, layer 3 onto that, and so on.",
        ],
      },
      {
        points: [
          "On a laser, Mark where the next layer goes lightly marks each layer with the outline of the one above. The marks get their own low-power operation.",
          "Fewer layers come out than you asked for if nothing in the picture is high enough for the top ones. Optimise picture first helps.",
          "Smallest piece leaves out fragments too small to handle.",
        ],
      },
    ],
  },

  // ---- Troubleshooting -----------------------------------------------------
  {
    id: "trouble-laser",
    title: "Laser results are not right",
    group: "When something goes wrong",
    machine: "laser",
    blocks: [
      {
        heading: "The engraving is faint",
        points: ["Raise the power or lower the speed.", "Check the focus, and clean the lens.", "Some woods simply mark lightly. Try a different piece."],
      },
      {
        heading: "The engraving is burnt, dark or smudged",
        points: ["Lower the power or raise the speed.", "Mask the surface with tape, and improve the extraction.", "For a photo that comes out as a dark block, add brightness in step 4 or press Optimise picture."],
      },
      {
        heading: "The cut does not go through",
        points: [
          "Add passes rather than slowing down.",
          "Check the focus was taken on this piece, and that it is lying flat.",
          "Use air assist, and lift the material off the bed.",
          "Check the material's real thickness. Plywood also has hard glue patches that resist cutting.",
        ],
      },
      {
        heading: "It came out mirrored, upside down or in the wrong place",
        points: [
          "Check the X0 Y0 corner in step 1 matches the machine: back-left for the WeCreat.",
          "Check the material is in that corner of the bed, and that step 2 has its real size.",
          "Run the small lopsided test described under Sending a job to the WeCreat.",
        ],
      },
    ],
  },
  {
    id: "trouble-router",
    title: "Router results are not right",
    group: "When something goes wrong",
    machine: "cnc",
    blocks: [
      {
        heading: "The bit broke",
        points: ["The cut was too deep or too fast for it. Lower Depth per pass first, then the speed.", "Make sure the bit is deep in the collet and the material cannot move.", "Fine bits need light cuts. Let a tapered bit do detailed work."],
      },
      {
        heading: "The wood is burning",
        points: ["The bit is rubbing instead of cutting. Raise the speed across the wood, or turn the router's dial down and enter the new speed in step 1.", "A dull or resin-coated bit burns. Clean or replace it."],
      },
      {
        heading: "Edges are fuzzy or torn",
        points: ["Try a down-cut bit for a clean top edge.", "Take a lighter final pass.", "Softwoods and plywood faces tear more than hardwoods. Sand or brush afterwards."],
      },
      {
        heading: "The job drifted out of place",
        points: [
          "The machine lost steps: it was pushed harder than its motors could manage. Lower the speed and depth per pass.",
          "Check the belts are snug and the wheels roll without wobble.",
          "Check the material did not shift, and that clamps were not struck.",
        ],
      },
      {
        heading: "The carving is taking too long",
        points: ["Choose Draft, or a larger bit.", "Make the piece smaller or the carving shallower.", "In Advanced, a larger Stepover is much faster, and shows as ridges you can sand."],
      },
    ],
  },
  {
    id: "trouble-app",
    title: "Kerf will not do what I want",
    group: "When something goes wrong",
    blocks: [
      {
        heading: "It says the job does not fit the bed",
        points: ["Something reaches past the machine's working area. In step 3, move or shrink it, or use Fill the material.", "Check the bed size and corner in step 1."],
      },
      {
        heading: "Something reaches past the edge of the material",
        points: ["Select it in step 3 and move or resize it, or correct the material's size in step 2.", "A cut-out's gap counts too: it runs outside everything on the material."],
      },
      {
        heading: "Going back cleared my job",
        points: ["A generated job belongs to the settings it was made from, so going back to an earlier step clears it. Your picture and settings are kept: press Generate again.", "The warning can be switched on or off at the bottom of step 1."],
      },
      {
        heading: "The assistant does nothing, or shows odd text",
        points: ["Open its settings with the cog and check a model and, for hosted models, a key are set.", "Small local models sometimes fail to use the app's tools. A larger model, or a hosted one, follows instructions better."],
      },
    ],
  },
];

/** The topic that explains the step the person is on. */
export function topicForStep(step: string, machine: MachineKind): string {
  switch (step) {
    case "material":
      return "material";
    case "design":
      return "make-sign";
    case "trace":
    case "operations":
      return machine === "laser" ? "laser-practice" : "router-practice";
    case "machine":
      return "router-running";
    default:
      return "steps";
  }
}
