/**
 * No pipeline in the URL (docs/16 §Application layout; docs/17 wave 4 O2):
 * with a token the page IS the picker — the pipelines this origin opened
 * recently, and the served root's file list one directory at a time (`GET
 * /api/files`, never `/api/project`: over a home root its walk is seconds
 * and lists what the picker must not show). Choosing one opens it in this
 * tab through the route (`history.pushState`, so Back returns to the
 * picker). Without a token the page can only explain how to get a URL:
 * `cicada serve` prints one with the token.
 */
import { useState } from "react";
import { browserStorage, readRecent } from "../state/recent";
import { openPipeline, routeSearch } from "../state/route";
import { FileBrowser } from "./FileBrowser";

export function Landing({ token }: { token?: string }) {
  const [recent] = useState(() => readRecent(browserStorage()));

  if (token === undefined) {
    return (
      <main className="landing">
        <h1>Cicada</h1>
        <p>
          This page needs the session token. Open the URL that <code>cicada serve</code> printed
          (<code>?token=…</code>) — the token is Jupyter-style: it lives in the URL, never in the page.
        </p>
      </main>
    );
  }
  return (
    <main className="landing" data-testid="landing">
      <h1>Cicada</h1>
      <p className="dim">
        Open a pipeline under the served root — Enter or a double-click opens it here; a directory opens
        in place.
      </p>
      {recent.length > 0 && (
        <section className="landing-section" data-testid="landing-recent">
          <h2>Recent</h2>
          <ul className="landing-recent">
            {recent.map((entry) => (
              <li key={entry}>
                <a
                  className="mono"
                  href={routeSearch({ token, pipeline: entry, view: "app" })}
                  onClick={(event) => {
                    // A plain click stays in this tab through the route; a
                    // modified click (new tab, new window) is the browser's.
                    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
                    event.preventDefault();
                    openPipeline(entry);
                  }}
                  data-testid={`landing-recent-${entry}`}
                >
                  {entry}
                </a>
              </li>
            ))}
          </ul>
        </section>
      )}
      <section className="landing-section">
        <h2>Files</h2>
        <FileBrowser token={token} onOpen={openPipeline} autoFocus />
      </section>
    </main>
  );
}
