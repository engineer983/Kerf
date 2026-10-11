/**
 * Saved projects, kept in this browser's own storage (IndexedDB) for this
 * site. They stay on this computer, in this browser.
 */
import { engine, enginePicture } from "./engine";

interface Meta {
  id: string;
  name: string;
  savedAt: string;
  thumbnail: string;
}

interface Saved {
  id: string;
  meta: Meta;
  settings: string;
  image: Uint8Array;
  depth: Uint8Array | null;
}

const STORE = "projects";

let opened: Promise<IDBDatabase> | null = null;

function database(): Promise<IDBDatabase> {
  opened ??= new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open("kerf", 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: "id" });
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(`This browser would not open Kerf's storage: ${request.error?.message ?? "unknown reason"}`);
  });
  opened.catch(() => (opened = null));
  return opened;
}

async function store<T>(mode: IDBTransactionMode, work: (projects: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await database();
  return new Promise<T>((resolve, reject) => {
    const transaction = db.transaction(STORE, mode);
    const request = work(transaction.objectStore(STORE));
    transaction.oncomplete = () => resolve(request.result);
    transaction.onerror = transaction.onabort = () =>
      reject(`This browser could not save or read the project: ${(transaction.error ?? request.error)?.message ?? "unknown reason"}`);
  });
}

async function saved(id: string): Promise<Saved> {
  const project = await store<Saved | undefined>("readonly", (projects) => projects.get(id));
  if (!project) throw "That project is no longer saved in this browser.";
  return project;
}

export async function saveProject({ meta, settings }: { meta: Meta; settings: string }): Promise<void> {
  const image = await enginePicture("image");
  const depth = await enginePicture("depth");
  await store("readwrite", (projects) =>
    projects.put({ id: meta.id, meta, settings, image, depth: depth.length ? depth : null } satisfies Saved),
  );
  // Asks the browser not to clear these to make room. It may say no, which changes nothing here.
  void navigator.storage?.persist?.().catch(() => false);
}

/** Saved projects, most recently saved first. */
export async function listProjects(): Promise<Meta[]> {
  const all = await store<Saved[]>("readonly", (projects) => projects.getAll());
  return all.map((project) => project.meta).sort((a, b) => b.savedAt.localeCompare(a.savedAt));
}

/** Makes a saved project's picture (and depth map) the one being worked on, and resolves to its settings. */
export async function openProject({ id }: { id: string }): Promise<string> {
  const project = await saved(id);
  await engine("load_image", {}, project.image);
  if (project.depth) await engine("set_depth_map", {}, project.depth);
  return project.settings;
}

export async function projectPicture({ id, which }: { id: string; which: "image" | "depth" }): Promise<ArrayBuffer> {
  const project = await saved(id);
  const bytes = which === "image" ? project.image : project.depth;
  if (!bytes) throw "That project has no depth map.";
  return bytes.slice().buffer as ArrayBuffer;
}

export async function deleteProject({ id }: { id: string }): Promise<void> {
  await store("readwrite", (projects) => projects.delete(id));
}