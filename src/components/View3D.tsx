import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import type { Simulation } from "../lib/simulation";

/** Vertices along the longer side of the carved surface. */
const SURFACE_VERTICES = 320;
const TOOL_LENGTH_MM = 30;

interface Scene3D {
  /** Redraws, refreshing the surface if the simulation has changed. */
  render: () => void;
  dispose: () => void;
}

function build(container: HTMLDivElement, sim: Simulation, bed: { width: number; height: number }): Scene3D {
  const { stock, thickness, material, kind, tool } = sim.setup;
  const style = getComputedStyle(container);

  const renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(Math.min(2, window.devicePixelRatio));
  container.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  scene.background = new THREE.Color(style.getPropertyValue("--bg").trim() || "#151413");
  scene.add(new THREE.HemisphereLight(0xffffff, 0x554433, 1.5));
  const sun = new THREE.DirectionalLight(0xffffff, 2.2);
  // Low and from the back left, so that carved edges cast visible shading.
  sun.position.set(-1.4, 1.1, -1.2);
  scene.add(sun);

  // Scene axes: X to the right, Y up, Z towards the front of the machine.
  // The stock's top is at Y = 0 and its centre is at the origin.
  const centre = { x: stock.x + stock.width / 2, y: stock.y + stock.height / 2 };

  const grid = new THREE.GridHelper(Math.max(bed.width, bed.height), Math.max(bed.width, bed.height) / 10, 0x666666, 0x3a3a3a);
  grid.position.set(bed.width / 2 - centre.x, -thickness - 0.05, bed.height / 2 - centre.y);
  scene.add(grid);

  // The top surface: flat for a laser, displaced by the height grid for a router.
  const carved = kind === "cnc" && sim.heights !== null;
  const scale = carved ? Math.min(1, SURFACE_VERTICES / Math.max(sim.gridWidth, sim.gridHeight)) : 0;
  const columns = carved ? Math.max(2, Math.round(sim.gridWidth * scale)) : 2;
  const rows = carved ? Math.max(2, Math.round(sim.gridHeight * scale)) : 2;
  const surface = new THREE.PlaneGeometry(stock.width, stock.height, columns - 1, rows - 1);
  surface.rotateX(-Math.PI / 2);
  // The simulation's picture already darkens carved areas with depth, which
  // keeps shallow reliefs readable on top of the scene lighting.
  const texture = new THREE.CanvasTexture(sim.canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = renderer.capabilities.getMaxAnisotropy();
  const top = new THREE.Mesh(surface, new THREE.MeshStandardMaterial({ map: texture, roughness: 0.85 }));
  scene.add(top);

  // Walls from the edge of the surface down to the bottom of the stock.
  const edge: number[] = [];
  for (let c = 0; c < columns; c++) edge.push(c);
  for (let r = 1; r < rows; r++) edge.push(r * columns + columns - 1);
  for (let c = columns - 2; c >= 0; c--) edge.push((rows - 1) * columns + c);
  for (let r = rows - 2; r >= 1; r--) edge.push(r * columns);
  const wallPositions = new Float32Array(edge.length * 6);
  const wallIndex: number[] = [];
  for (let i = 0; i < edge.length; i++) {
    const next = (i + 1) % edge.length;
    wallIndex.push(i * 2, next * 2, i * 2 + 1, next * 2, next * 2 + 1, i * 2 + 1);
  }
  const walls = new THREE.BufferGeometry();
  walls.setAttribute("position", new THREE.BufferAttribute(wallPositions, 3));
  walls.setIndex(wallIndex);
  const [r, g, b] = material.base;
  const wallColor = new THREE.Color().setRGB(r / 255, g / 255, b / 255, THREE.SRGBColorSpace).multiplyScalar(0.8);
  scene.add(new THREE.Mesh(walls, new THREE.MeshStandardMaterial({ color: wallColor, roughness: 0.9, side: THREE.DoubleSide })));

  const radius = kind === "cnc" ? tool.diameter / 2 : 0.35;
  const toolGeometry = new THREE.CylinderGeometry(radius, radius, TOOL_LENGTH_MM, 24);
  // Put the cylinder's bottom at the tool position.
  toolGeometry.translate(0, TOOL_LENGTH_MM / 2 + (kind === "cnc" && tool.ballNose ? radius : 0), 0);
  const toolMaterial =
    kind === "cnc"
      ? new THREE.MeshStandardMaterial({ color: 0xb8bcc4, metalness: 0.7, roughness: 0.35 })
      : new THREE.MeshBasicMaterial({ color: 0xff3b2f, transparent: true, opacity: 0.75 });
  const toolMesh = new THREE.Group();
  toolMesh.add(new THREE.Mesh(toolGeometry, toolMaterial));
  if (kind === "cnc" && tool.ballNose) {
    const tip = new THREE.Mesh(new THREE.SphereGeometry(radius, 24, 16), toolMaterial);
    tip.position.y = radius;
    toolMesh.add(tip);
  }
  scene.add(toolMesh);

  const camera = new THREE.PerspectiveCamera(35, 1, 1, 20000);
  const span = Math.max(stock.width, stock.height);
  camera.position.set(-span * 0.35, span * 0.95, span * 1.25);
  const controls = new OrbitControls(camera, renderer.domElement);
  controls.enableDamping = true;
  controls.maxPolarAngle = Math.PI / 2 - 0.02;
  controls.target.set(0, -thickness / 2, 0);

  let seen = -1;
  const refreshSurface = () => {
    const positions = surface.getAttribute("position") as THREE.BufferAttribute;
    if (carved) {
      const heights = sim.heights!;
      for (let r = 0; r < rows; r++) {
        const gy = Math.min(sim.gridHeight - 1, Math.round((r / (rows - 1)) * (sim.gridHeight - 1)));
        for (let c = 0; c < columns; c++) {
          const gx = Math.min(sim.gridWidth - 1, Math.round((c / (columns - 1)) * (sim.gridWidth - 1)));
          positions.setY(r * columns + c, Math.max(-thickness, heights[gy * sim.gridWidth + gx]));
        }
      }
      positions.needsUpdate = true;
      surface.computeVertexNormals();
    }
    texture.needsUpdate = true;
    for (let i = 0; i < edge.length; i++) {
      const v = edge[i];
      wallPositions.set([positions.getX(v), positions.getY(v), positions.getZ(v)], i * 6);
      wallPositions.set([positions.getX(v), -thickness, positions.getZ(v)], i * 6 + 3);
    }
    walls.getAttribute("position").needsUpdate = true;
    walls.computeVertexNormals();
  };

  const resize = () => {
    const { clientWidth, clientHeight } = container;
    if (clientWidth === 0 || clientHeight === 0) return;
    renderer.setSize(clientWidth, clientHeight);
    camera.aspect = clientWidth / clientHeight;
    camera.updateProjectionMatrix();
  };
  const observer = new ResizeObserver(resize);
  observer.observe(container);
  resize();

  return {
    render: () => {
      if (seen !== sim.version) {
        seen = sim.version;
        refreshSurface();
      }
      toolMesh.position.set(sim.head[0] - centre.x, sim.head[2], sim.head[1] - centre.y);
      controls.update();
      renderer.render(scene, camera);
    },
    dispose: () => {
      observer.disconnect();
      controls.dispose();
      renderer.dispose();
      renderer.domElement.remove();
      scene.traverse((object) => {
        if (object instanceof THREE.Mesh) {
          object.geometry.dispose();
          for (const m of [object.material].flat()) m.dispose();
        }
      });
      texture.dispose();
    },
  };
}

interface View3DProps {
  simulation: Simulation;
  bed: { width: number; height: number };
}

/** An orbitable 3D view of the material as the job has left it so far. */
export function View3D({ simulation, bed }: View3DProps) {
  const container = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const scene = build(container.current!, simulation, bed);
    let frame = 0;
    const loop = () => {
      scene.render();
      frame = requestAnimationFrame(loop);
    };
    loop();
    return () => {
      cancelAnimationFrame(frame);
      scene.dispose();
    };
  }, [simulation, bed]);

  return (
    <div
      ref={container}
      className="absolute inset-0 cursor-default"
      // Dragging here orbits the 3D view; it must not also pan the 2D canvas.
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onWheel={(e) => e.stopPropagation()}
    />
  );
}
