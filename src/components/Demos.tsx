import type { ReactNode } from "react";
import type { DemoId } from "../lib/hints";

// Small looping diagrams that show what a setting does. They are drawn in a
// 120 × 64 box and animated with SVG's own animation elements.

const wood = "var(--muted)";
const cut = "var(--accent)";
const ink = "var(--fg)";
const DUR = "4s";

const loop = { dur: DUR, repeatCount: "indefinite" } as const;

/** A router bit seen from the side, with its tip at (0, 0). */
function Bit({ children }: { children?: ReactNode }) {
  return (
    <g>
      <rect x={-3} y={-26} width={6} height={26} rx={1} fill={ink} opacity={0.75} />
      {children}
    </g>
  );
}

function Label({ x, y, children }: { x: number; y: number; children: ReactNode }) {
  return (
    <text x={x} y={y} fontSize={7} fill={ink} opacity={0.7} textAnchor="middle" fontFamily="inherit">
      {children}
    </text>
  );
}

const DEMOS: Record<DemoId, ReactNode> = {
  // Top view: the bit sweeps rows; the gap between rows is the stepover.
  stepover: (
    <>
      <rect x={8} y={8} width={104} height={48} rx={3} fill={wood} opacity={0.25} />
      {[16, 28, 40].map((y) => (
        <line key={y} x1={14} x2={106} y1={y} y2={y} stroke={cut} strokeWidth={5} strokeLinecap="round" opacity={0.35} />
      ))}
      <circle r={4} fill={cut}>
        <animateMotion {...loop} path="M14 16H106V28H14V40H106" />
      </circle>
      <path d="M112 16v12M109 16h6M109 28h6" stroke={ink} strokeWidth={1} fill="none" />
    </>
  ),
  // Side view: the bit goes a step deeper on each trip.
  depthPerPass: (
    <>
      <rect x={8} y={30} width={104} height={28} fill={wood} opacity={0.25} />
      {[0, 1, 2].map((i) => (
        <rect key={i} x={20} y={30 + i * 7} width={80} height={7} fill={cut} opacity={0}>
          <animate attributeName="opacity" {...loop} values={`0;0;0.45;0.45`} keyTimes={`0;${0.1 + i * 0.3};${0.3 + i * 0.3};1`} />
        </rect>
      ))}
      <g>
        <animateTransform attributeName="transform" type="translate" {...loop}
          values="20,37;100,37;20,44;100,44;20,51;100,51;100,51" keyTimes="0;0.3;0.31;0.6;0.61;0.9;1" />
        <Bit />
      </g>
    </>
  ),
  // Top view: one line, with the bit on it, inside it and outside it.
  cutSide: (
    <>
      <rect x={30} y={16} width={60} height={34} fill={wood} opacity={0.2} stroke={ink} strokeWidth={1} strokeDasharray="3 2" />
      <circle r={5} fill={cut} opacity={0.85}>
        <animateMotion {...loop} path="M30 16H90V50H30Z" />
      </circle>
      <circle r={5} fill="none" stroke={cut} strokeWidth={1.5}>
        <animateMotion {...loop} path="M35 21H85V45H35Z" />
      </circle>
      <circle r={5} fill="none" stroke={cut} strokeWidth={1.5} strokeDasharray="2 2">
        <animateMotion {...loop} path="M25 11H95V55H25Z" />
      </circle>
      <Label x={60} y={36}>on · inside · outside</Label>
    </>
  ),
  // Side view of a through cut, with bridges left at the bottom.
  tabs: (
    <>
      <rect x={8} y={24} width={104} height={24} fill={wood} opacity={0.25} />
      <path d="M8 24H112V48H84V42H72V48H48V42H36V48H8Z" fill={cut} opacity={0.3} />
      <g>
        <animateTransform attributeName="transform" type="translate" {...loop}
          values="12,48;36,48;36,42;48,42;48,48;72,48;72,42;84,42;84,48;108,48"
          keyTimes="0;0.2;0.25;0.35;0.4;0.6;0.65;0.75;0.8;1" />
        <Bit />
      </g>
      <Label x={42} y={58}>tab</Label>
      <Label x={78} y={58}>tab</Label>
    </>
  ),
  // The cut line moving out from, and in to, the artwork's edge.
  offset: (
    <>
      <rect x={40} y={20} width={40} height={24} fill={wood} opacity={0.35} />
      <rect fill="none" stroke={cut} strokeWidth={1.5} strokeDasharray="4 2">
        <animate attributeName="x" {...loop} values="40;26;26;46;40" />
        <animate attributeName="y" {...loop} values="20;6;6;26;20" />
        <animate attributeName="width" {...loop} values="40;68;68;28;40" />
        <animate attributeName="height" {...loop} values="24;52;52;12;24" />
      </rect>
      <Label x={60} y={35}>artwork</Label>
    </>
  ),
  // Engraved lines close together, then far apart.
  lineInterval: (
    <>
      <rect x={8} y={8} width={104} height={48} rx={3} fill={wood} opacity={0.25} />
      {[0, 1, 2, 3, 4, 5, 6, 7].map((i) => (
        <line key={i} x1={14} x2={106} stroke={cut} strokeWidth={2}>
          <animate attributeName="y1" {...loop} values={`${14 + i * 3};${14 + i * 3};${12 + i * 6};${12 + i * 6};${14 + i * 3}`} />
          <animate attributeName="y2" {...loop} values={`${14 + i * 3};${14 + i * 3};${12 + i * 6};${12 + i * 6};${14 + i * 3}`} />
        </line>
      ))}
    </>
  ),
  // The same grey made two ways: scattered dots, and an even lighter burn.
  shading: (
    <>
      <rect x={8} y={10} width={48} height={36} fill={wood} opacity={0.25} />
      <rect x={64} y={10} width={48} height={36} fill={wood} opacity={0.25} />
      {Array.from({ length: 60 }, (_, i) => (
        <rect key={i} x={10 + ((i * 37) % 44)} y={12 + ((i * 23) % 32)} width={2} height={2} fill={cut} />
      ))}
      <rect x={64} y={10} width={48} height={36} fill={cut}>
        <animate attributeName="opacity" {...loop} values="0.15;0.6;0.15" />
      </rect>
      <Label x={32} y={56}>dots</Label>
      <Label x={88} y={56}>varying power</Label>
    </>
  ),
  // A grey ramp; everything left of the moving line turns black.
  threshold: (
    <>
      <defs>
        <linearGradient id="demo-ramp">
          <stop offset="0" stopColor="#111" />
          <stop offset="1" stopColor="#eee" />
        </linearGradient>
      </defs>
      <rect x={8} y={8} width={104} height={20} fill="url(#demo-ramp)" />
      <rect x={8} y={36} width={104} height={20} fill="#eee" />
      <rect x={8} y={36} height={20} fill="#111">
        <animate attributeName="width" {...loop} values="26;78;26" />
      </rect>
      <line y1={4} y2={60} stroke={cut} strokeWidth={1.5}>
        <animate attributeName="x1" {...loop} values="34;86;34" />
        <animate attributeName="x2" {...loop} values="34;86;34" />
      </line>
    </>
  ),
  // The bit lifts clear, moves across, and comes back down.
  safeHeight: (
    <>
      <rect x={8} y={40} width={104} height={18} fill={wood} opacity={0.25} />
      <rect x={52} y={32} width={16} height={8} fill={ink} opacity={0.5} />
      <path d="M24 40V18H96V40" fill="none" stroke={cut} strokeWidth={1} strokeDasharray="3 2" />
      <g>
        <animateTransform attributeName="transform" type="translate" {...loop} values="24,40;24,18;96,18;96,40;96,40" keyTimes="0;0.25;0.65;0.9;1" />
        <Bit />
      </g>
      <Label x={60} y={14}>safe height</Label>
    </>
  ),
  // The same picture carved with light high, then with dark high.
  raisedAreas: (
    <>
      <defs>
        <linearGradient id="demo-shade">
          <stop offset="0" stopColor="#222" />
          <stop offset="0.5" stopColor="#eee" />
          <stop offset="1" stopColor="#222" />
        </linearGradient>
      </defs>
      <rect x={20} y={6} width={80} height={10} fill="url(#demo-shade)" />
      <path fill={wood} opacity={0.45}>
        <animate attributeName="d" {...loop}
          values="M20 58V44Q60 16 100 44V58Z;M20 58V44Q60 16 100 44V58Z;M20 58V26Q60 54 100 26V58Z;M20 58V26Q60 54 100 26V58Z;M20 58V44Q60 16 100 44V58Z"
          keyTimes="0;0.4;0.5;0.9;1" />
      </path>
      <Label x={60} y={26}>picture</Label>
    </>
  ),
  // Scaling evenly, against stretching one way.
  proportions: (
    <>
      <rect fill={cut} opacity={0.5} rx={2}>
        <animate attributeName="x" {...loop} values="50;30;50;20;50" />
        <animate attributeName="y" {...loop} values="22;12;22;22;22" />
        <animate attributeName="width" {...loop} values="20;60;20;80;20" />
        <animate attributeName="height" {...loop} values="20;40;20;20;20" />
      </rect>
      <Label x={32} y={60}>kept</Label>
      <Label x={90} y={60}>stretched</Label>
    </>
  ),
  cornerRadius: (
    <rect x={24} y={12} width={72} height={40} fill={wood} fillOpacity={0.25} stroke={cut} strokeWidth={2}>
      <animate attributeName="rx" {...loop} values="0;18;0" />
    </rect>
  ),
  // A tapered ball nose: a thick shank narrowing to a small ball.
  taper: (
    <>
      <rect x={8} y={50} width={104} height={8} fill={wood} opacity={0.25} />
      <path d="M48 4H72V14L62 48Q60 51 58 48L48 14Z" fill={ink} opacity={0.75} />
      <path d="M60 4V54" stroke={cut} strokeWidth={0.8} strokeDasharray="2 2" />
      <path d="M72 14L62 48" stroke={cut} strokeWidth={1.5} />
      <circle cx={60} cy={48} r={2.2} fill="none" stroke={cut} strokeWidth={1} />
      <Label x={90} y={32}>taper</Label>
      <Label x={36} y={50}>tip</Label>
    </>
  ),
  plunge: (
    <>
      <rect x={8} y={36} width={104} height={22} fill={wood} opacity={0.25} />
      <rect x={56} y={36} width={8} fill={cut} opacity={0.45}>
        <animate attributeName="height" {...loop} values="0;14;14" keyTimes="0;0.7;1" />
      </rect>
      <g>
        <animateTransform attributeName="transform" type="translate" {...loop} values="60,30;60,50;60,50" keyTimes="0;0.7;1" />
        <Bit />
      </g>
    </>
  ),
  // A sharp edge softening.
  blur: (
    <>
      <defs>
        <filter id="demo-blur">
          <feGaussianBlur>
            <animate attributeName="stdDeviation" {...loop} values="0;4;0" />
          </feGaussianBlur>
        </filter>
      </defs>
      <rect x={8} y={8} width={104} height={48} fill="#eee" />
      <circle cx={60} cy={32} r={16} fill="#111" filter="url(#demo-blur)" />
    </>
  ),
  // The same line burned again and again, darker each time.
  passes: (
    <>
      <rect x={8} y={8} width={104} height={48} rx={3} fill={wood} opacity={0.25} />
      <line x1={20} x2={100} y1={32} y2={32} stroke={cut} strokeWidth={5} strokeLinecap="round">
        <animate attributeName="opacity" {...loop} values="0.2;0.2;0.5;0.5;0.9;0.9" keyTimes="0;0.3;0.34;0.63;0.67;1" />
      </line>
      <circle r={3} fill={ink}>
        <animateMotion dur="1.33s" repeatCount="indefinite" path="M20 32H100" />
      </circle>
    </>
  ),
  // Brightness as height, against true shape from estimated depth.
  heightFrom: (
    <>
      <circle cx={32} cy={24} r={14} fill="#888" />
      <circle cx={32} cy={24} r={6} fill="#222" />
      <circle cx={88} cy={24} r={14} fill="#888" />
      <circle cx={88} cy={24} r={6} fill="#222" />
      <path d="M14 58V50Q20 42 26 50L29 56Q32 58 35 56L38 50Q44 42 50 50V58Z" fill={wood} opacity={0.5} />
      <path d="M70 58V52Q88 34 106 52V58Z" fill={wood} opacity={0.5} />
      <Label x={32} y={46}>brightness</Label>
      <Label x={88} y={46}>depth</Label>
    </>
  ),
};

export function Demo({ id }: { id: DemoId }) {
  return (
    <svg viewBox="0 0 120 64" className="w-full rounded-md border border-line bg-bg" role="img" aria-hidden="true">
      {DEMOS[id]}
    </svg>
  );
}
