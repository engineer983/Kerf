import { useEffect } from "react";
import { Trash2 } from "lucide-react";
import { isTauri } from "../lib/ipc";
import { useProject } from "../state/project";
import { Section } from "./controls";

function when(savedAt: string): string {
  const date = new Date(savedAt);
  return Number.isNaN(date.getTime())
    ? ""
    : date.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/** Earlier projects, each shown by a picture of its result, to open again. */
export function Gallery() {
  const projects = useProject((s) => s.projects);
  const current = useProject((s) => s.projectId);
  const { refreshProjects, openProject, deleteProject } = useProject.getState();

  useEffect(() => {
    if (isTauri()) void refreshProjects();
  }, [refreshProjects]);

  if (!isTauri()) return null;

  return (
    <Section title="Your projects">
      {projects.length === 0 ? (
        <p className="text-muted">
          Projects are kept here once you generate G-code or press Save, so you can come back to
          them.
        </p>
      ) : (
        <ul className="grid grid-cols-2 gap-2">
          {projects.map((project) => (
            <li key={project.id} className="group relative">
              <button
                onClick={() => void openProject(project.id)}
                title={`Open ${project.name}`}
                className={`flex w-full flex-col overflow-hidden rounded-lg border text-left transition-colors ${
                  project.id === current ? "border-accent" : "border-line hover:border-muted"
                }`}
              >
                <img src={project.thumbnail} alt="" className="aspect-square w-full bg-bg object-cover" />
                <span className="truncate px-2 pt-1.5 text-[12px]">{project.name}</span>
                <span className="px-2 pb-1.5 text-[11px] text-muted">{when(project.savedAt)}</span>
              </button>
              <button
                onClick={() => void deleteProject(project.id)}
                aria-label={`Delete ${project.name}`}
                title={`Delete ${project.name} from your projects`}
                className="absolute top-1 right-1 rounded-md bg-surface/90 p-1 text-muted opacity-0 transition-opacity group-hover:opacity-100 hover:text-accent focus:opacity-100"
              >
                <Trash2 size={13} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}
