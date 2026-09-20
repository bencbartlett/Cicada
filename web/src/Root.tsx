/**
 * The screen the route asks for (docs/16 §Application layout; docs/17 wave
 * 4 O2/O3): no token → the landing's explanation; a token alone → the
 * picker; a pipeline → the docked app; `view=viewport` → the pop-out (the
 * viewport alone, a declared observer). Also the three things every screen
 * shares: the theme on the document, the tab's title, and the tooltip layer
 * (every `title` in the app, shown after 250 ms — docs/16 §Theme).
 */
import { useEffect } from "react";
import { App } from "./App";
import { Landing } from "./panels/Landing";
import { titleFor, useRoute } from "./state/route";
import { useCicada } from "./state/store";
import { TooltipLayer } from "./TooltipLayer";
import { ViewportOnly } from "./ViewportOnly";

export function Root() {
  const route = useRoute((s) => s.route);
  const theme = useCicada((s) => s.settings.theme);

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  useEffect(() => {
    document.title = titleFor(route.pipeline, route.view);
  }, [route.pipeline, route.view]);

  const screen =
    route.token === undefined ? (
      <Landing token={undefined} />
    ) : route.pipeline === undefined ? (
      <Landing token={route.token} />
    ) : route.view === "viewport" ? (
      <ViewportOnly />
    ) : (
      <App />
    );
  return (
    <>
      {screen}
      <TooltipLayer />
    </>
  );
}
