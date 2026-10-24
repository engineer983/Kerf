# Ideas

Rough ideas for Kerf, waiting to be planned. To plan one, ask Claude to use the **dreamer** agent on it; the dreamer writes a plan into this folder (one file per idea) that a developer can build from.

| # | Idea | Status | Plan |
|---|------|--------|------|
| 1 | Camera view of the bed | planned | [camera-view.md](camera-view.md) |
| 2 | Auto-centre the design on the material | idea | — |
| 3 | Control the connected machine (X, Y, Z, homing) | idea | — |

## 1. Camera view of the bed

Show what a camera pointed at the bed can see. A still picture is enough to start with, with a button to take a new one of whatever is on the bed right now.

## 2. Auto-centre the design on the material

Find the piece of material on the bed and centre the design on it automatically, then let the person nudge it into place by hand.

Probably needs idea 1 first, since the camera is how Kerf would see where the material is.

## 3. Control the connected machine

Move the connected machine in X, Y and Z and home it.

Note: the machine panel (`src/components/MachinePanel.tsx`, `src-tauri/src/machine.rs`) already has X/Y/Z jog buttons and a Home button. The plan should start by working out what's still missing (for example, continuous jogging, go to a position, setting the work origin, or a keyboard shortcut).
