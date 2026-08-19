/**
 * No pipeline in the URL: list the project's pipelines (needs the token) or
 * explain how to get a URL. `cicada serve <file.cic>` prints a URL with both.
 */
import { useEffect, useState } from "react";

interface ProjectInfo {
  project: string;
  pipelines: string[];
  default: string | null;
  open: string[];
  engine: string;
}

export function Landing({ token }: { token?: string }) {
  const [info, setInfo] = useState<ProjectInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (token === undefined) return;
    fetch("/api/project", { headers: { "X-Cicada-Token": token } })
      .then(async (response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}: ${await response.text()}`);
        return (await response.json()) as ProjectInfo;
      })
      .then(setInfo)
      .catch((e: unknown) => setError(String(e)));
  }, [token]);

  if (token === undefined) {
    return (
      <main className="landing">
        <h1>Cicada</h1>
        <p>
          This page needs the session token. Open the URL that <code>cicada serve</code> printed
          (<code>?token=…&amp;pipeline=…</code>) — the token is Jupyter-style: it lives in the URL,
          never in the page.
        </p>
      </main>
    );
  }
  return (
    <main className="landing" data-testid="landing">
      <h1>Cicada</h1>
      {error !== null && <p style={{ color: "var(--error)" }}>{error}</p>}
      {info === null && error === null && <p className="dim">loading project…</p>}
      {info !== null && (
        <>
          <p>
            Project <code>{info.project}</code> · {info.engine}
          </p>
          <p>Open a pipeline:</p>
          <ul>
            {info.pipelines.map((p) => (
              <li key={p}>
                <a href={`?token=${encodeURIComponent(token)}&pipeline=${encodeURIComponent(p)}`}>
                  {p}
                </a>
                {info.open.includes(p) && <span className="badge accent">open</span>}
              </li>
            ))}
          </ul>
          {info.pipelines.length === 0 && (
            <p className="dim">
              No <code>.cic</code> files in this project yet — create one and reload.
            </p>
          )}
        </>
      )}
    </main>
  );
}
